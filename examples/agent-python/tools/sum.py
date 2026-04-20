#!/usr/bin/env python3
"""Process tool: JSON stdin {"a": int, "b": int} -> JSON stdout {"result": int}."""
import json
import sys


def main() -> None:
    data = json.load(sys.stdin)
    result = int(data["a"]) + int(data["b"])
    json.dump({"result": result}, sys.stdout)
    sys.stdout.write("\n")


if __name__ == "__main__":
    main()
