import os
import re
import sys
import tomllib

ROOT = os.path.abspath(os.path.join(os.path.dirname(__file__), "..", ".."))
BUDGET = os.path.join(ROOT, "test-budget.toml")
SKIP = {"target", ".git", "vendor", "node_modules"}
TEST = re.compile(r"^\s*#\[test\]")


def packages():
    found = []
    for directory, dirs, files in os.walk(ROOT):
        dirs[:] = [d for d in dirs if d not in SKIP]
        if "Cargo.toml" in files:
            with open(os.path.join(directory, "Cargo.toml"), encoding="utf-8") as manifest:
                if "[package]" in manifest.read():
                    found.append(os.path.relpath(directory, ROOT).replace(os.sep, "/"))
    return sorted(found, key=len, reverse=True)


def owner(path, roots):
    for root in roots:
        if root == "." or path == root or path.startswith(root + "/"):
            return root
    return None


def counts():
    roots = packages()
    tally = {root: 0 for root in roots}
    for directory, dirs, files in os.walk(ROOT):
        dirs[:] = [d for d in dirs if d not in SKIP]
        for name in files:
            if not name.endswith(".rs"):
                continue
            path = os.path.join(directory, name)
            root = owner(os.path.relpath(path, ROOT).replace(os.sep, "/"), roots)
            if root is None:
                continue
            with open(path, encoding="utf-8") as source:
                tally[root] += sum(1 for line in source if TEST.match(line))
    return {root: n for root, n in tally.items() if n}


def write(tally):
    with open(BUDGET, "w", encoding="utf-8") as out:
        for root in sorted(tally):
            out.write(f'"{root}" = {tally[root]}\n')


def main():
    tally = counts()
    if sys.argv[1:] == ["--write"]:
        write(tally)
        print(f"wrote {sum(tally.values())} tests across {len(tally)} packages to test-budget.toml")
        return 0
    with open(BUDGET, "rb") as budget_file:
        budget = tomllib.load(budget_file)
    wrong = []
    for root in sorted(set(tally) | set(budget)):
        have, allowed = tally.get(root, 0), budget.get(root, 0)
        if have > allowed:
            wrong.append(f"{root}: {have} tests, budget {allowed}. Fold the new cases into an existing scenario, table or property, or raise the budget in this change and say why.")
        elif have < allowed:
            wrong.append(f"{root}: {have} tests, budget {allowed}. Lower the budget to {have}.")
    for line in wrong:
        print(line)
    if wrong:
        print("Run `python3 .github/scripts/test_budget.py --write` to write the current counts.")
        return 1
    print(f"{sum(tally.values())} tests, within budget")
    return 0


if __name__ == "__main__":
    sys.exit(main())
