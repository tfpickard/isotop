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

# Safe to rerun after a failure: reuse the project, add only the missing field, and let
# item-add return the existing item for an issue that is already on the board.
existing = [p for p in gh("project", "list", "--owner", OWNER, "--format", "json")["projects"]
            if p["title"] == "isotop"]
project = existing[0] if existing else gh("project", "create", "--owner", OWNER,
                                          "--title", "isotop", "--format", "json")
number, project_id = str(project["number"]), project["id"]


def fields_of(project_number):
    return {f["name"]: f for f in gh("project", "field-list", project_number, "--owner", OWNER,
                                     "--format", "json")["fields"]}


fields = fields_of(number)
if "Kind" not in fields:
    gh("project", "field-create", number, "--owner", OWNER, "--name", "Kind",
       "--data-type", "SINGLE_SELECT", "--single-select-options", ",".join([*KINDS, "other"]))
    fields = fields_of(number)
try:
    gh("project", "link", number, "--owner", OWNER, "--repo", REPO)
except subprocess.CalledProcessError:
    pass  # already linked

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
