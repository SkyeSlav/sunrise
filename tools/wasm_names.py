#!/usr/bin/env python3
"""Print the sorted import and export names of a .wasm member, one per line.

    python3 tools/wasm_names.py sunrise-wasm-std.wasm > names-linux.txt

A wasm mod imports the symbols of the members before it, so two builds of the std member (or a mod) agree
only if these lists do: CI builds the std member on Linux and on Windows and diffs the lists. Lines are
`import MODULE.NAME KIND` and `export NAME KIND`; the output is the same on every host.
"""
import sys

KINDS = {0: "func", 1: "table", 2: "memory", 3: "global", 4: "tag"}


def leb(data, at):
    value = shift = 0
    while True:
        byte = data[at]
        at += 1
        value |= (byte & 0x7F) << shift
        if byte < 0x80:
            return value, at
        shift += 7


def name(data, at):
    size, at = leb(data, at)
    return data[at:at + size].decode("utf-8"), at + size


def limits(data, at):
    flags, at = leb(data, at)
    _, at = leb(data, at)
    if flags & 1:
        _, at = leb(data, at)
    return at


def import_type(data, at, kind):
    """Skip the descriptor of an import of `kind`."""
    if kind == 0:
        return leb(data, at)[1]
    if kind == 1:
        return limits(data, at + 1)
    if kind == 2:
        return limits(data, at)
    if kind == 3:
        return at + 2
    if kind == 4:
        return leb(data, at + 1)[1]
    raise ValueError(f"unknown import kind {kind}")


def names(data):
    """The sorted `import …` and `export …` lines of the module `data`."""
    if data[:8] != b"\0asm\1\0\0\0":
        raise ValueError("not a wasm module")
    lines = []
    at = 8
    while at < len(data):
        section = data[at]
        size, at = leb(data, at + 1)
        end = at + size
        if section in (2, 7):
            count, at = leb(data, at)
            for _ in range(count):
                if section == 2:
                    module, at = name(data, at)
                    field, at = name(data, at)
                    kind = data[at]
                    at = import_type(data, at + 1, kind)
                    lines.append(f"import {module}.{field} {KINDS[kind]}")
                else:
                    field, at = name(data, at)
                    kind = data[at]
                    _, at = leb(data, at + 1)
                    lines.append(f"export {field} {KINDS[kind]}")
        at = end
    return sorted(lines)


def main(argv):
    if len(argv) != 2:
        print("usage: wasm_names.py <member.wasm>", file=sys.stderr)
        return 2
    with open(argv[1], "rb") as member:
        print("\n".join(names(member.read())))
    return 0


if __name__ == "__main__":
    sys.exit(main(sys.argv))
