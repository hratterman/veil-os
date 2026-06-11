#!/usr/bin/env python3
"""Hand-assemble two small WASM modules (no toolchain needed):
  HELLO.WASM   - imports wasi fd_write, prints a string via _start
  COMPUTE.WASM - exports compute(n): a hot arithmetic loop (JIT target),
                 and fib(n): recursive (interpreter path).
"""
import struct
import sys


def uleb(n):
    out = bytearray()
    while True:
        b = n & 0x7F
        n >>= 7
        if n:
            out.append(b | 0x80)
        else:
            out.append(b)
            return bytes(out)


def sleb(n):
    out = bytearray()
    more = True
    while more:
        b = n & 0x7F
        n >>= 7
        if (n == 0 and not (b & 0x40)) or (n == -1 and (b & 0x40)):
            more = False
        else:
            b |= 0x80
        out.append(b)
    return bytes(out)


def vec(items):
    return uleb(len(items)) + b"".join(items)


def section(sid, body):
    return bytes([sid]) + uleb(len(body)) + body


def name(s):
    b = s.encode()
    return uleb(len(b)) + b


# opcodes
I32_CONST = 0x41
LOCAL_GET = 0x20
LOCAL_SET = 0x21
LOCAL_TEE = 0x22
I32_ADD = 0x6A
I32_MUL = 0x6C
I32_SUB = 0x6B
I32_LT_S = 0x48
I32_GE_S = 0x4E
CALL = 0x10
BR_IF = 0x0D
BR = 0x0C
LOOP = 0x03
BLOCK = 0x02
IF = 0x04
ELSE = 0x05
END = 0x0B
DROP = 0x1A
RETURN = 0x0F


def i32c(n):
    return bytes([I32_CONST]) + sleb(n)


def hello():
    msg = b"Hello from WebAssembly, running on Veil OS!\n"
    # memory layout: [0..4] iov.base=8, [4..8] iov.len=len(msg), [8..] msg
    data = struct.pack("<II", 8, len(msg)) + msg
    NW = 200  # nwritten scratch ptr

    types = vec([
        # type 0: (i32,i32,i32,i32) -> i32   (fd_write)
        bytes([0x60]) + vec([bytes([0x7F])] * 4) + vec([bytes([0x7F])]),
        # type 1: () -> ()                    (_start)
        bytes([0x60]) + vec([]) + vec([]),
    ])
    imports = vec([
        name("wasi_snapshot_preview1") + name("fd_write") + bytes([0x00]) + uleb(0),
    ])
    funcs = vec([uleb(1)])  # _start has type 1
    mems = vec([bytes([0x00]) + uleb(1)])  # 1 page, no max
    exports = vec([
        name("_start") + bytes([0x00]) + uleb(1),   # func idx 1 (import is 0)
        name("memory") + bytes([0x02]) + uleb(0),
    ])
    body = (i32c(1) + i32c(0) + i32c(1) + i32c(NW) + bytes([CALL]) + uleb(0)
            + bytes([DROP, END]))
    code_entry = uleb(0) + body  # 0 local decls
    code = vec([uleb(len(code_entry)) + code_entry])
    data_sec = vec([uleb(0) + bytes([I32_CONST]) + sleb(0) + bytes([END]) + uleb(len(data)) + data])

    return (b"\0asm" + struct.pack("<I", 1)
            + section(1, types) + section(2, imports) + section(3, funcs)
            + section(5, mems) + section(7, exports) + section(10, code)
            + section(11, data_sec))


def compute():
    # compute(n): local i=1, acc=0; loop { acc += i*i; i++; if i<n br } ; acc
    # locals: param n (0), i (1), acc (2)
    compute_body = (
        i32c(1) + bytes([LOCAL_SET]) + uleb(1)           # i = 1
        + bytes([LOOP, 0x40])                             # loop (void)
        + bytes([LOCAL_GET]) + uleb(2)                    # acc
        + bytes([LOCAL_GET]) + uleb(1) + bytes([LOCAL_GET]) + uleb(1) + bytes([I32_MUL])  # i*i
        + bytes([I32_ADD]) + bytes([LOCAL_SET]) + uleb(2)  # acc = acc + i*i
        + bytes([LOCAL_GET]) + uleb(1) + i32c(1) + bytes([I32_ADD]) + bytes([LOCAL_SET]) + uleb(1)  # i++
        + bytes([LOCAL_GET]) + uleb(1) + bytes([LOCAL_GET]) + uleb(0) + bytes([I32_LT_S])  # i < n
        + bytes([BR_IF]) + uleb(0)                        # continue loop
        + bytes([END])                                    # end loop
        + bytes([LOCAL_GET]) + uleb(2)                    # return acc
        + bytes([END])
    )
    compute_locals = uleb(1) + uleb(2) + bytes([0x7F])    # 2 i32 locals (i, acc)
    compute_entry = compute_locals + compute_body

    # fib(n): if n<2 return n else fib(n-1)+fib(n-2)
    fib_body = (
        bytes([LOCAL_GET]) + uleb(0) + i32c(2) + bytes([I32_LT_S])
        + bytes([IF, 0x7F])                               # if (result i32)
        + bytes([LOCAL_GET]) + uleb(0)
        + bytes([ELSE])
        + bytes([LOCAL_GET]) + uleb(0) + i32c(1) + bytes([I32_SUB]) + bytes([CALL]) + uleb(1)
        + bytes([LOCAL_GET]) + uleb(0) + i32c(2) + bytes([I32_SUB]) + bytes([CALL]) + uleb(1)
        + bytes([I32_ADD])
        + bytes([END])                                    # end if
        + bytes([END])
    )
    fib_entry = uleb(0) + fib_body                        # no locals

    types = vec([bytes([0x60]) + vec([bytes([0x7F])]) + vec([bytes([0x7F])])])  # (i32)->i32
    funcs = vec([uleb(0), uleb(0)])                       # compute=type0, fib=type0
    exports = vec([
        name("compute") + bytes([0x00]) + uleb(0),
        name("fib") + bytes([0x00]) + uleb(1),
    ])
    code = vec([
        uleb(len(compute_entry)) + compute_entry,
        uleb(len(fib_entry)) + fib_entry,
    ])
    return (b"\0asm" + struct.pack("<I", 1)
            + section(1, types) + section(3, funcs) + section(7, exports) + section(10, code))


if __name__ == "__main__":
    out = sys.argv[1] if len(sys.argv) > 1 else "assets"
    open(f"{out}/hello.wasm", "wb").write(hello())
    open(f"{out}/compute.wasm", "wb").write(compute())
    print(f"wrote {out}/hello.wasm ({len(hello())} B), {out}/compute.wasm ({len(compute())} B)")
