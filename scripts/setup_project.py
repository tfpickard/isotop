#!/usr/bin/env python3
"""Create the isotop GitHub Project and load every issue into it.

Run locally, once, after:  gh auth refresh -s project
Usage: python3 setup_project.py [--dry-run]
"""
import json
import subprocess
import sys

OWNER, REPO = "tfpickard", "tfpickard/isotop"
KINDS = ["roadmap", "view", "building-block", "prerequisite", "infra",
         "enhancement", "bug", "documentation"]
DRY = "--dry-run" in sys.argv


def gh(*args):
    out = subprocess.run(["gh", *args], check=True, capture_output=True, text=True).stdout
    return json.loads(out) if out.strip().startswith(("{", "[")) else out


def kind(labels):
    names = {label["name"] for label in labels}
    return next((k for k in KINDS if k in names), "other")


issues = gh("issue", "list", "-R", REPO, "--state", "all", "--limit", "1000",
            "--json", "number,url,state,labels")
print(f"{len(issues)} issues to add")
if DRY:
    sys.exit(0)

project = gh("project", "create", "--owner", OWNER, "--title", "isotop", "--format", "json")
number, project_id = str(project["number"]), project["id"]
gh("project", "field-create", number, "--owner", OWNER, "--name", "Kind",
   "--data-type", "SINGLE_SELECT", "--single-select-options", ",".join([*KINDS, "other"]))
gh("project", "link", number, "--owner", OWNER, "--repo", REPO)

fields = {f["name"]: f for f in gh("project", "field-list", number, "--owner", OWNER,
                                   "--format", "json")["fields"]}
status, kinds = fields["Status"], fields["Kind"]
status_option = {o["name"]: o["id"] for o in status["options"]}
kind_option = {o["name"]: o["id"] for o in kinds["options"]}

for issue in issues:
    item = gh("project", "item-add", number, "--owner", OWNER, "--url", issue["url"],
              "--format", "json")["id"]
    for field, option in (
        (status, status_option["Done" if issue["state"] == "CLOSED" else "Todo"]),
        (kinds, kind_option[kind(issue["labels"])]),
    ):
        gh("project", "item-edit", "--id", item, "--project-id", project_id,
           "--field-id", field["id"], "--single-select-option-id", option)
    print(f"#{issue['number']} added")
print(project["url"])
