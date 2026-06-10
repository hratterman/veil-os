#!/usr/bin/env python3
"""Raw-ethernet peer for the M12/M13 proof.

QEMU's dgram netdev hands us the guest's ethernet frames as UDP datagrams
(one frame per datagram) and injects whatever frames we send back. We play
the role of the gateway host (10.0.2.2):

  M12: observe the guest's hand-crafted 0x88b5 frame on the wire, and
       answer its ARP probe (that reply is the frame the guest prints).
  M13: ARP-request the guest's IP and validate the reply, then send real
       ICMP echo requests and byte-validate the echo replies.
  M14 (udp half): send a UDP datagram to the echo port, validate the echo.

Usage: netpeer.py <listen-port> <qemu-port>   (exit 0 = all checks passed)
"""
import socket
import struct
import sys
import time

OUR_MAC = bytes.fromhex("52550a000202")
OUR_IP = bytes([10, 0, 2, 2])
GUEST_IP = bytes([10, 0, 2, 15])
GUEST_MAC = bytes.fromhex("525400123456")

passed = 0
failed = 0


def check(name, ok, detail=""):
    global passed, failed
    print(f"{'PASS' if ok else 'FAIL'}: {name}" + (f" ({detail})" if detail else ""))
    if ok:
        passed += 1
    else:
        failed += 1


def csum(data):
    if len(data) % 2:
        data += b"\x00"
    s = sum(struct.unpack(f">{len(data)//2}H", data))
    while s >> 16:
        s = (s & 0xFFFF) + (s >> 16)
    return (~s) & 0xFFFF


def eth(dst, ethertype, payload):
    f = dst + OUR_MAC + struct.pack(">H", ethertype) + payload
    return f + b"\x00" * max(0, 60 - len(f))


def arp_request(target_ip):
    return eth(b"\xff" * 6, 0x0806,
               struct.pack(">HHBBH", 1, 0x0800, 6, 4, 1)
               + OUR_MAC + OUR_IP + b"\x00" * 6 + target_ip)


def arp_reply(dst_mac, dst_ip):
    return eth(dst_mac, 0x0806,
               struct.pack(">HHBBH", 1, 0x0800, 6, 4, 2)
               + OUR_MAC + OUR_IP + dst_mac + dst_ip)


def ipv4(proto, payload):
    hdr = struct.pack(">BBHHHBBH4s4s", 0x45, 0, 20 + len(payload), 1, 0x4000,
                      64, proto, 0, OUR_IP, GUEST_IP)
    hdr = hdr[:10] + struct.pack(">H", csum(hdr)) + hdr[12:]
    return eth(GUEST_MAC, 0x0800, hdr + payload)


def icmp_echo(ident, seq, payload):
    icmp = struct.pack(">BBHHH", 8, 0, 0, ident, seq) + payload
    icmp = icmp[:2] + struct.pack(">H", csum(icmp)) + icmp[4:]
    return ipv4(1, icmp)


def udp(sport, dport, payload):
    pseudo = OUR_IP + GUEST_IP + struct.pack(">BBH", 0, 17, 8 + len(payload))
    hdr = struct.pack(">HHHH", sport, dport, 8 + len(payload), 0)
    s = csum(pseudo + hdr + payload) or 0xFFFF
    return ipv4(17, struct.pack(">HHHH", sport, dport, 8 + len(payload), s) + payload)


class Peer:
    def __init__(self, listen_port, qemu_port):
        self.sock = socket.socket(socket.AF_INET, socket.SOCK_DGRAM)
        self.sock.bind(("127.0.0.1", listen_port))
        self.sock.settimeout(0.2)
        self.qemu = ("127.0.0.1", qemu_port)
        self.seen_m12 = False
        self.seen_guest_arp = False

    def send(self, frame):
        self.sock.sendto(frame, self.qemu)

    def pump(self, want=None, timeout=10.0):
        """Service passive duties until `want(frame)` matches or timeout."""
        deadline = time.time() + timeout
        while time.time() < deadline:
            try:
                frame, _ = self.sock.recvfrom(4096)
            except socket.timeout:
                continue
            if len(frame) < 14:
                continue
            ethertype = struct.unpack(">H", frame[12:14])[0]
            if ethertype == 0x88B5 and b"VEIL M12" in frame:
                self.seen_m12 = True
            if ethertype == 0x0806 and len(frame) >= 42:
                oper = struct.unpack(">H", frame[20:22])[0]
                tpa = frame[38:42]
                if oper == 1 and tpa == OUR_IP:  # who-has 10.0.2.2
                    self.seen_guest_arp = True
                    self.send(arp_reply(frame[6:12], frame[28:32]))
            if want and want(frame):
                return frame
        return None


def main():
    peer = Peer(int(sys.argv[1]), int(sys.argv[2]))
    print("netpeer: listening, waiting for the guest to boot...")

    # --- M12: passive observations while the guest boots ---
    peer.pump(want=lambda f: peer.seen_m12 and peer.seen_guest_arp, timeout=60)
    check("M12 tx: hand-crafted 0x88b5 frame observed on the wire", peer.seen_m12)
    check("M12 rx: guest ARP probe observed (we answered it)", peer.seen_guest_arp)

    # --- M13: ARP both directions ---
    peer.send(arp_request(GUEST_IP))

    def is_arp_reply(f):
        return (struct.unpack(">H", f[12:14])[0] == 0x0806 and len(f) >= 42
                and struct.unpack(">H", f[20:22])[0] == 2
                and f[28:32] == GUEST_IP and f[38:42] == OUR_IP)

    reply = peer.pump(want=is_arp_reply, timeout=5)
    check("M13 arp: guest answered who-has 10.0.2.15", reply is not None,
          f"sha={reply[22:28].hex(':')}" if reply else "timeout")
    if reply:
        check("M13 arp: reply carries the guest's mac", reply[22:28] == GUEST_MAC)

    # --- M13: ICMP echo, three rounds, byte-validated ---
    for seq in range(1, 4):
        payload = (f"veil-ping-{seq}-".encode() * 4)[:48]
        peer.send(icmp_echo(0x4242, seq, payload))

        def is_echo_reply(f, seq=seq, payload=payload):
            if len(f) < 14 + 20 + 8 or struct.unpack(">H", f[12:14])[0] != 0x0800:
                return False
            ihl = (f[14] & 0xF) * 4
            if f[23] != 1 or f[26:30] != GUEST_IP or f[30:34] != OUR_IP:
                return False
            icmp = f[14 + ihl:]
            return (icmp[0] == 0 and icmp[1] == 0
                    and struct.unpack(">HH", icmp[4:8]) == (0x4242, seq)
                    and icmp[8:8 + len(payload)] == payload)

        r = peer.pump(want=is_echo_reply, timeout=5)
        check(f"M13 icmp: echo reply seq={seq} (id, seq, payload all match)", r is not None)
        if r:
            ihl = (r[14] & 0xF) * 4
            total = struct.unpack(">H", r[16:18])[0]
            icmp = r[14 + ihl:14 + total]
            check(f"M13 icmp: reply seq={seq} checksum valid", csum(icmp) == 0)

    # --- M14 (udp): echo service ---
    peer.send(udp(40000, 7, b"veil udp echo proof"))

    def is_udp_echo(f):
        if len(f) < 14 + 20 + 8 or struct.unpack(">H", f[12:14])[0] != 0x0800:
            return False
        ihl = (f[14] & 0xF) * 4
        if f[23] != 17 or f[26:30] != GUEST_IP:
            return False
        u = f[14 + ihl:]
        return (struct.unpack(">HH", u[0:4]) == (7, 40000)
                and u[8:8 + 19] == b"veil udp echo proof")

    r = peer.pump(want=is_udp_echo, timeout=5)
    check("M14 udp: datagram echoed back from :7", r is not None)

    print(f"netpeer: {passed} passed, {failed} failed")
    return 1 if failed else 0


if __name__ == "__main__":
    sys.exit(main())
