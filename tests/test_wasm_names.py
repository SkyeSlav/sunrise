"""The name lists CI compares between hosts: read from a module built byte by byte."""
import unittest

from tools import wasm_names


def string(text):
    raw = text.encode()
    return bytes([len(raw)]) + raw


def section(number, body):
    return bytes([number, len(body)]) + body


# imports: env.b (func type 0), env.a (memory, min 1), env.g (global i32 const); exports: z (func 0), y (global 0)
MODULE = (
    b"\0asm\1\0\0\0"
    + section(1, bytes([1, 0x60, 0, 0]))
    + section(2, bytes([3])
              + string("env") + string("b") + bytes([0, 0])
              + string("env") + string("a") + bytes([2, 0, 1])
              + string("env") + string("g") + bytes([3, 0x7F, 0]))
    + section(7, bytes([2]) + string("z") + bytes([0, 0]) + string("y") + bytes([3, 0]))
)


class WasmNamesTests(unittest.TestCase):
    def test_lists_imports_and_exports_sorted(self):
        self.assertEqual(wasm_names.names(MODULE), [
            "export y global",
            "export z func",
            "import env.a memory",
            "import env.b func",
            "import env.g global",
        ])

    def test_section_order_in_the_file_does_not_change_the_list(self):
        swapped = (MODULE[:8] + section(7, bytes([1]) + string("z") + bytes([0, 0]))
                   + section(2, bytes([1]) + string("env") + string("b") + bytes([0, 0])))
        self.assertEqual(wasm_names.names(swapped), ["export z func", "import env.b func"])

    def test_refuses_a_file_that_is_not_wasm(self):
        with self.assertRaises(ValueError):
            wasm_names.names(b"MZ\x90\0")


if __name__ == "__main__":
    unittest.main()
