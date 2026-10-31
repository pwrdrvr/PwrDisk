#!/usr/bin/env python3
"""Read native AT-SPI metadata from an explicitly named fixture process.

Run inside the same desktop/session bus as a11y-fixture. This diagnostic
does not invoke actions, synthesize input, or start a screen reader.
"""

import argparse
import json
import time

import pyatspi


def inspect(node, depth=0, remaining=None):
    if remaining is None:
        remaining = [5000]
    if remaining[0] == 0 or depth > 32:
        return
    remaining[0] -= 1
    row = {
        "depth": depth,
        "role": node.getRoleName(),
        "name": node.name,
        "description": node.description,
        "states": [str(state) for state in node.getState().getStates()],
        "relations": [str(rel.getRelationType()) for rel in node.getRelationSet()],
    }
    try:
        action = node.queryAction()
        row["actions"] = [action.getName(index) for index in range(action.nActions)]
    except NotImplementedError:
        pass
    try:
        value = node.queryValue()
        row["value"] = value.currentValue
    except NotImplementedError:
        pass
    print(json.dumps(row, ensure_ascii=False), flush=True)
    for index in range(node.childCount):
        child = node.getChildAtIndex(index)
        if child is not None:
            inspect(child, depth + 1, remaining)


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--pid", required=True, type=int)
    args = parser.parse_args()
    deadline = time.monotonic() + 20
    while time.monotonic() < deadline:
        desktop = pyatspi.Registry.getDesktop(0)
        for app in desktop:
            if app.get_process_id() == args.pid and app.childCount:
                inspect(app)
                return
        time.sleep(0.25)
    raise SystemExit("No native AT-SPI application found for the supplied fixture PID")


if __name__ == "__main__":
    main()
