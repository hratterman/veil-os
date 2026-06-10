#!/usr/bin/env python3
"""Verify the M14 pass criterion from a packet capture: the TCP handshake,
two-way data, and orderly teardown on the echo port, plus the M12 raw
frame. Reads the pcap QEMU's filter-dump object wrote (guest's wire view:
guest ip 10.0.2.15, peer 10.0.2.2).

Usage: checkpcap.py <file.pcap> <port>
"""
import struct
import sys

GUEST = bytes([10, 0, 2, 15])


def parse_pcap(path):
    with open(path, "rb") as f:
        data = f.read()
    magic = struct.unpack("<I", data[0:4])[0]
    if magic == 0xA1B2C3D4:
        endian = "<"
    elif struct.unpack(">I", data[0:4])[0] == 0xA1B2C3D4:
        endian = ">"
    else:
        sys.exit(f"not a pcap file: {path}")
    off = 24
    while off + 16 <= len(data):
        incl = struct.unpack(endian + "I", data[off + 8:off + 12])[0]
        yield data[off + 16:off + 16 + incl]
        off += 16 + incl


def main():
    path, port = sys.argv[1], int(sys.argv[2])
    saw_m12 = False
    segs = []  # (dir 'g'|'p', flags, seq, ack, paylen, payload)
    for frame in parse_pcap(path):
        if len(frame) < 14:
            continue
        if struct.unpack(">H", frame[12:14])[0] == 0x88B5 and b"VEIL M12" in frame:
            saw_m12 = True
            continue
        if struct.unpack(">H", frame[12:14])[0] != 0x0800 or len(frame) < 34:
            continue
        ihl = (frame[14] & 0xF) * 4
        if frame[23] != 6:
            continue
        src, dst = frame[26:30], frame[30:34]
        tcp = frame[14 + ihl:14 + struct.unpack(">H", frame[16:18])[0]]
        if len(tcp) < 20:
            continue
        sport, dport = struct.unpack(">HH", tcp[0:4])
        if port not in (sport, dport):
            continue
        seq, ack = struct.unpack(">II", tcp[4:12])
        doff = (tcp[12] >> 4) * 4
        flags = tcp[13]
        payload = tcp[doff:]
        segs.append(("g" if src == GUEST else "p", flags, seq, ack, payload))

    failures = []

    def check(name, ok):
        print(f"{'PASS' if ok else 'FAIL'}: pcap: {name}")
        if not ok:
            failures.append(name)

    FIN, SYN, ACK = 0x01, 0x02, 0x10
    syns = [s for s in segs if s[0] == "p" and s[1] & SYN and not s[1] & ACK]
    synacks = [s for s in segs if s[0] == "g" and s[1] & SYN and s[1] & ACK]
    check("client SYN seen", bool(syns))
    check("guest SYN-ACK seen", bool(synacks))
    if syns and synacks:
        check("SYN-ACK acknowledges SYN's seq+1",
              synacks[0][3] == (syns[0][2] + 1) & 0xFFFFFFFF)
        ack3 = [s for s in segs if s[0] == "p" and s[1] & ACK and not s[1] & SYN
                and s[3] == (synacks[0][2] + 1) & 0xFFFFFFFF]
        check("handshake ACK acknowledges SYN-ACK's seq+1", bool(ack3))
    g_data = b"".join(s[4] for s in segs if s[0] == "g")
    p_data = b"".join(s[4] for s in segs if s[0] == "p")
    check("guest sent data (greeting + echo)", b"VEIL TCP ECHO" in g_data and b"echo:" in g_data)
    check("client sent data", len(p_data) > 0)
    g_fins = [s for s in segs if s[0] == "g" and s[1] & FIN]
    p_fins = [s for s in segs if s[0] == "p" and s[1] & FIN]
    check("guest FIN seen (active close)", bool(g_fins))
    check("client FIN seen", bool(p_fins))
    if g_fins:
        fin_seq = (g_fins[0][2] + len(g_fins[0][4]) + 1) & 0xFFFFFFFF
        check("client ACKed guest's FIN",
              any(s[0] == "p" and s[1] & ACK and s[3] == fin_seq for s in segs))
    if p_fins:
        fin_seq = (p_fins[0][2] + len(p_fins[0][4]) + 1) & 0xFFFFFFFF
        check("guest ACKed client's FIN",
              any(s[0] == "g" and s[1] & ACK and s[3] == fin_seq for s in segs))
    check("M12 hand-crafted 0x88b5 frame in capture", saw_m12)

    print(f"checkpcap: {len(segs)} tcp segments on :{port}, "
          f"{'ALL OK' if not failures else f'{len(failures)} FAILURES'}")
    return 1 if failures else 0


if __name__ == "__main__":
    sys.exit(main())
