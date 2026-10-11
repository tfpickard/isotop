//! Where remote addresses are: a MaxMind-format city database (GeoLite2, DB-IP Lite) when one
//! is installed, else the legacy country database many Linux distributions ship. Home comes from
//! the system time zone. All lookups are local; nothing is sent anywhere.

use std::collections::HashMap;
use std::fs;
use std::net::IpAddr;
use std::path::{Path, PathBuf};

use crate::model::{Location, Place};
use crate::places::{COUNTRIES, LEGACY_CODES};

/// Legacy GeoIP country databases store leaves at or above this record value.
const COUNTRY_BEGIN: usize = 16_776_960;

/// The legacy country databases for IPv4 and IPv6 addresses.
const LEGACY_DATABASES: [&str; 2] = ["/usr/share/GeoIP/GeoIP.dat", "/usr/share/GeoIP/GeoIPv6.dat"];

/// Where distributions and Homebrew (Apple Silicon, then Intel) put city databases.
const CITY_DATABASES: [&str; 7] = [
    "/usr/share/GeoIP/GeoLite2-City.mmdb",
    "/var/lib/GeoIP/GeoLite2-City.mmdb",
    "/usr/share/GeoIP/dbip-city-lite.mmdb",
    "/usr/local/share/GeoIP/GeoLite2-City.mmdb",
    "/opt/homebrew/share/GeoIP/GeoLite2-City.mmdb",
    "/opt/homebrew/var/GeoIP/GeoLite2-City.mmdb",
    "/usr/local/var/GeoIP/GeoLite2-City.mmdb",
];

pub struct Geo {
    candidates: Vec<PathBuf>,
    /// The legacy country databases for IPv4 and IPv6, in that order.
    legacy_paths: [PathBuf; 2],
    loaded: bool,
    city: Option<maxminddb::Reader<Vec<u8>>>,
    legacy: [Option<Vec<u8>>; 2],
    cache: HashMap<IpAddr, Location>,
    /// Which database answers lookups, for the status strip.
    pub source: String,
}

impl Geo {
    /// Finds the databases but reads none: a city database can be over 100 MB, so it is only
    /// loaded by the first lookup.
    pub fn open(explicit: Option<&Path>) -> Self {
        let mut candidates: Vec<PathBuf> = explicit.map(Path::to_path_buf).into_iter().collect();
        if let Some(home) = std::env::var_os("HOME") {
            let own = Path::new(&home).join(".local/share/isotop");
            if let Ok(entries) = fs::read_dir(own) {
                let mut found: Vec<PathBuf> = entries
                    .flatten()
                    .map(|e| e.path())
                    .filter(|p| p.extension().is_some_and(|x| x == "mmdb"))
                    .collect();
                found.sort();
                candidates.extend(found);
            }
        }
        candidates.extend(CITY_DATABASES.iter().map(PathBuf::from));
        Self::with_databases(candidates, LEGACY_DATABASES.map(PathBuf::from))
    }

    /// A locator over exactly these databases: city candidates in order of preference, then the
    /// legacy country files for IPv4 and IPv6.
    pub(crate) fn with_databases(candidates: Vec<PathBuf>, legacy_paths: [PathBuf; 2]) -> Self {
        Self {
            candidates,
            legacy_paths,
            loaded: false,
            city: None,
            legacy: [None, None],
            cache: HashMap::new(),
            source: "loading the GeoIP database".into(),
        }
    }

    fn load(&mut self) {
        self.loaded = true;
        let city = self
            .candidates
            .iter()
            .find_map(|path| Some((path, maxminddb::Reader::open_readfile(path).ok()?)));
        self.legacy = [
            fs::read(&self.legacy_paths[0]).ok(),
            fs::read(&self.legacy_paths[1]).ok(),
        ];
        self.source = match (&city, &self.legacy) {
            (Some((path, reader)), _) => {
                let name = path.file_name().map_or_else(
                    || path.display().to_string(),
                    |n| n.to_string_lossy().into_owned(),
                );
                let kind = format!("{name} {}", reader.metadata().database_type).to_lowercase();
                // DB-IP Lite is licensed CC BY 4.0, which requires this exact attribution.
                if kind.contains("dbip") || kind.contains("db-ip") {
                    "IP Geolocation by DB-IP".into()
                } else {
                    format!("cities from {name}")
                }
            }
            (None, [Some(_), _] | [_, Some(_)]) => {
                "countries from the system GeoIP database".into()
            }
            (None, _) => "no GeoIP database (see --geoip)".into(),
        };
        self.city = city.map(|(_, reader)| reader);
    }

    /// Where `address` is, or why it has no place. Private addresses are answered without
    /// reading any database.
    pub fn locate(&mut self, address: IpAddr) -> Location {
        let address = unmapped(address);
        if !public(address) {
            return Location::Private;
        }
        if let Some(known) = self.cache.get(&address) {
            return known.clone();
        }
        if !self.loaded {
            self.load();
        }
        let family = usize::from(address.is_ipv6());
        let found = if self.city.is_none() && self.legacy[family].is_none() {
            Location::NoDatabase
        } else {
            self.city_lookup(address)
                .or_else(|| self.country_lookup(address))
                .map_or(Location::Unlisted, Location::Known)
        };
        self.cache.insert(address, found.clone());
        found
    }

    fn city_lookup(&self, address: IpAddr) -> Option<Place> {
        let record: maxminddb::geoip2::City =
            self.city.as_ref()?.lookup(address).ok()?.decode().ok()??;
        let latitude = record.location.latitude? as f32;
        let longitude = record.location.longitude? as f32;
        let country = record.country.iso_code.unwrap_or("");
        let name = match record.city.names.english {
            Some(city) if !country.is_empty() => format!("{city}, {country}"),
            Some(city) => city.to_owned(),
            None => record.country.names.english.unwrap_or(country).to_owned(),
        };
        Some(Place {
            latitude,
            longitude,
            name,
        })
    }

    fn country_lookup(&self, address: IpAddr) -> Option<Place> {
        let index = match address {
            IpAddr::V4(v4) => legacy_country(self.legacy[0].as_deref()?, &v4.octets()),
            IpAddr::V6(v6) => legacy_country(self.legacy[1].as_deref()?, &v6.octets()),
        }?;
        country(LEGACY_CODES.get(index)?)
    }
}

/// A country's label point by ISO code.
pub fn country(code: &str) -> Option<Place> {
    let &(_, latitude, longitude, name) = COUNTRIES.iter().find(|c| c.0 == code)?;
    Some(Place {
        latitude,
        longitude,
        name: name.into(),
    })
}

/// Walks a legacy GeoIP binary tree (3-byte little-endian records, most significant bit first)
/// to a country index; 0 means unknown.
fn legacy_country(data: &[u8], address: &[u8]) -> Option<usize> {
    let bits = address.len() * 8;
    let mut offset = 0;
    for i in 0..bits {
        let bit = (address[i / 8] >> (7 - i % 8) & 1) as usize;
        let at = offset * 6 + bit * 3;
        let record = data.get(at..at + 3)?;
        let next = record[0] as usize | (record[1] as usize) << 8 | (record[2] as usize) << 16;
        if next >= COUNTRY_BEGIN {
            return Some(next - COUNTRY_BEGIN).filter(|&index| index > 0);
        }
        offset = next;
    }
    None
}

/// An IPv4-mapped IPv6 address as the IPv4 address it carries; any other address as it is.
fn unmapped(address: IpAddr) -> IpAddr {
    match address {
        IpAddr::V6(v6) => v6.to_ipv4_mapped().map_or(address, IpAddr::V4),
        v4 => v4,
    }
}

/// Globally routable addresses only: private, loopback, link-local, CGNAT and ULA ranges have
/// no geography. An IPv4-mapped IPv6 address is judged as the IPv4 address it carries.
pub(crate) fn public(address: IpAddr) -> bool {
    match unmapped(address) {
        IpAddr::V4(v4) => {
            let [a, b, ..] = v4.octets();
            !(v4.is_private()
                || v4.is_loopback()
                || v4.is_link_local()
                || v4.is_unspecified()
                || v4.is_broadcast()
                || v4.is_multicast()
                || (a == 100 && (64..128).contains(&b)))
        }
        IpAddr::V6(v6) => {
            let first = v6.segments()[0];
            !(v6.is_loopback()
                || v6.is_unspecified()
                || v6.is_multicast()
                || (first & 0xfe00) == 0xfc00
                || (first & 0xffc0) == 0xfe80)
        }
    }
}

/// Where this machine is: an explicit location, else the coordinates of the system time zone
/// from the time zone database.
pub fn home(explicit: Option<(f32, f32)>) -> Option<Place> {
    if let Some((latitude, longitude)) = explicit {
        return Some(Place {
            latitude,
            longitude,
            name: "home".into(),
        });
    }
    let zone = fs::read_link("/etc/localtime").ok()?;
    let zone = zone.to_str()?.split("zoneinfo/").nth(1)?.to_owned();
    let table = fs::read_to_string("/usr/share/zoneinfo/zone1970.tab")
        .or_else(|_| fs::read_to_string("/usr/share/zoneinfo/zone.tab"))
        .ok()?;
    let line = table
        .lines()
        .filter(|l| !l.starts_with('#'))
        .find(|l| l.split('\t').nth(2) == Some(zone.as_str()))?;
    let (latitude, longitude) = iso6709(line.split('\t').nth(1)?)?;
    Some(Place {
        latitude,
        longitude,
        name: zone.rsplit('/').next()?.replace('_', " "),
    })
}

/// Parses `+DDMM[SS]+DDDMM[SS]` coordinates.
fn iso6709(text: &str) -> Option<(f32, f32)> {
    let split = text[1..].find(['+', '-'])? + 1;
    let angle = |part: &str, degrees: usize| -> Option<f32> {
        let sign = if part.starts_with('-') { -1.0 } else { 1.0 };
        let digits = &part[1..];
        let d: f32 = digits.get(..degrees)?.parse().ok()?;
        let m: f32 = digits.get(degrees..degrees + 2)?.parse().ok()?;
        let s: f32 = digits
            .get(degrees + 2..)
            .filter(|s| !s.is_empty())
            .map_or(Some(0.0), |s| s.parse().ok())?;
        Some(sign * (d + m / 60.0 + s / 3600.0))
    };
    Some((angle(&text[..split], 2)?, angle(&text[split..], 3)?))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn coordinates_and_ranges() {
        let (lat, lon) = iso6709("+394421-1045903").unwrap();
        assert!((lat - 39.739).abs() < 0.01 && (lon + 104.984).abs() < 0.01);
        assert!(!public("10.1.2.3".parse().unwrap()));
        assert!(!public("100.64.0.1".parse().unwrap()));
        assert!(!public("fd00::1".parse().unwrap()));
        assert!(!public("::ffff:10.1.2.3".parse().unwrap()));
        assert!(public("::ffff:8.8.8.8".parse().unwrap()));
        assert!(public("8.8.8.8".parse().unwrap()));
        assert_eq!(country("JP").unwrap().name, "Japan");
        const PSEUDO: [&str; 6] = ["--", "AP", "EU", "A1", "A2", "O1"];
        for code in LEGACY_CODES.iter().filter(|c| !PSEUDO.contains(c)) {
            assert!(
                country(code).is_some(),
                "legacy code {code} has no label point"
            );
        }
    }

    /// A locator over no city database and the given legacy files, which are written to a
    /// directory of their own and read by the first public lookup.
    fn locator(name: &str, legacy: [Option<Vec<u8>>; 2]) -> Geo {
        let directory =
            std::env::temp_dir().join(format!("isotop-geo-{}-{name}", std::process::id()));
        fs::create_dir_all(&directory).unwrap();
        let paths = ["v4.dat", "v6.dat"].map(|file| directory.join(file));
        for (path, data) in paths.iter().zip(legacy) {
            match data {
                Some(data) => fs::write(path, data).unwrap(),
                None => drop(fs::remove_file(path)),
            }
        }
        Geo::with_databases(Vec::new(), paths)
    }

    /// A legacy tree of one node whose two children are leaves holding `record`.
    fn leaves(record: usize) -> Vec<u8> {
        let bytes = record.to_le_bytes();
        [&bytes[..3], &bytes[..3]].concat()
    }

    #[test]
    fn private_addresses_are_private_without_reading_a_database() {
        let mut geo = locator("private", [None, None]);
        for address in [
            "10.1.2.3",
            "100.64.0.1",
            "192.168.0.9",
            "127.0.0.1",
            "fd00::1",
            "::ffff:10.1.2.3",
            "::ffff:100.64.0.1",
        ] {
            assert_eq!(geo.locate(address.parse().unwrap()), Location::Private);
        }
        assert!(geo.source.contains("loading"), "{}", geo.source);
        assert!(!geo.loaded);
    }

    #[test]
    fn a_public_address_without_any_database_says_there_is_none() {
        let mut geo = locator("none", [None, None]);
        assert_eq!(geo.locate("8.8.8.8".parse().unwrap()), Location::NoDatabase);
        assert_eq!(
            geo.locate("2001:4860:4860::8888".parse().unwrap()),
            Location::NoDatabase
        );
        assert_eq!(geo.source, "no GeoIP database (see --geoip)");
    }

    #[test]
    fn an_address_the_database_does_not_list_is_unlisted() {
        let mut geo = locator("unlisted", [Some(leaves(COUNTRY_BEGIN)), None]);
        assert_eq!(geo.locate("8.8.8.8".parse().unwrap()), Location::Unlisted);
    }

    #[test]
    fn an_address_the_database_lists_is_known() {
        let index = LEGACY_CODES.iter().position(|&code| code == "JP").unwrap();
        let mut geo = locator("known", [Some(leaves(COUNTRY_BEGIN + index)), None]);
        match geo.locate("::ffff:202.12.27.33".parse().unwrap()) {
            Location::Known(place) => assert_eq!(place.name, "Japan"),
            other => panic!("{other:?}"),
        }
    }

    #[test]
    fn only_the_missing_family_has_no_database() {
        let mut geo = locator("family", [Some(leaves(COUNTRY_BEGIN)), None]);
        assert_eq!(geo.locate("8.8.8.8".parse().unwrap()), Location::Unlisted);
        assert_eq!(
            geo.locate("2001:4860:4860::8888".parse().unwrap()),
            Location::NoDatabase
        );
        let mut geo = locator("family-v6", [None, Some(leaves(COUNTRY_BEGIN))]);
        assert_eq!(geo.locate("8.8.8.8".parse().unwrap()), Location::NoDatabase);
        assert_eq!(
            geo.locate("2001:4860:4860::8888".parse().unwrap()),
            Location::Unlisted
        );
    }

    #[test]
    fn legacy_database_matches_libgeoip_where_installed() {
        let Ok(data) = fs::read("/usr/share/GeoIP/GeoIP.dat") else {
            return;
        };
        for (ip, code) in [
            ("8.8.8.8", "US"),
            ("81.2.69.142", "GB"),
            ("202.12.27.33", "JP"),
            ("77.88.8.8", "RU"),
            ("200.160.2.3", "BR"),
        ] {
            let address: std::net::Ipv4Addr = ip.parse().unwrap();
            let index = legacy_country(&data, &address.octets()).unwrap();
            assert_eq!(LEGACY_CODES[index], code, "{ip}");
        }
    }
}
