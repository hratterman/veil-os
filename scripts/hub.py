#!/usr/bin/env python3
"""A tiny UDP reflector hub: a virtual ethernet switch for the M20
two-instance demo.

QEMU's -netdev dgram is point-to-point, and on this host its delivery is
one-directional between two raw-bridged guests; socket mcast doesn't route
on macOS. So instead each Veil instance dgram-tunnels its ethernet frames
to this hub, and the hub forwards every frame it receives to all the OTHER
instances it has heard from. That makes broadcast (which M20 chat uses)
reach everyone, symmetrically — the same job the M21 VPS relay does, just
on localhost.

Usage: hub.py <listen-port>
"""
import socket
import sys


def main():
    port = int(sys.argv[1])
    s = socket.socket(socket.AF_INET, socket.SOCK_DGRAM)
    s.setsockopt(socket.SOL_SOCKET, socket.SO_REUSEADDR, 1)
    s.bind(("127.0.0.1", port))
    peers = set()
    while True:
        data, addr = s.recvfrom(65535)
        peers.add(addr)
        for p in peers:
            if p != addr:
                s.sendto(data, p)


if __name__ == "__main__":
    main()
