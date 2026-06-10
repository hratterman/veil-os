#!/usr/bin/env bash
# M12-M15 proof gauntlet, no root required.
#
# Phase 1 (dgram netdev): a Python raw-ethernet peer observes the guest's
#   hand-crafted frame (M12 tx), answers its ARP probe (M12 rx), then
#   sends real ARP/ICMP/UDP packets and byte-validates every reply (M13 +
#   the UDP half of M14).
# Phase 2 (slirp + hostfwd + filter-dump pcap): the Mac's own nc talks to
#   the TCP echo service (M14), curl and the Mac's real default browser
#   fetch the site off the FAT16 disk (M15), and the pcap proves the
#   handshake + orderly teardown packet-by-packet.
set -u
export PATH="$HOME/.cargo/bin:$PATH"
cd "$(dirname "$0")/.."

scripts/mkdisk.sh || exit 2
KERNEL=target/aarch64-unknown-none/debug/veil
mkdir -p shots
FAILED=0

note() { printf '\n=== %s ===================================================\n' "$1"; }
result() { # name ok
    if [ "$2" -eq 0 ]; then echo "PASS: $1"; else echo "FAIL: $1"; FAILED=1; fi
}
wait_serial() { # file needle timeout_s
    for _ in $(seq 1 $((${3:-30} * 10))); do
        grep -q "$2" "$1" 2>/dev/null && return 0
        sleep 0.1
    done
    return 1
}

# ---------------------------------------------------------------------------
note "phase 1: raw frames / ARP / ICMP / UDP against a host raw-ethernet peer"
S1=/tmp/veil-net1-serial.log
rm -f "$S1" /tmp/veil-netpeer.log
python3 scripts/netpeer.py 23001 23000 >/tmp/veil-netpeer.log 2>&1 &
PEER=$!
qemu-system-aarch64 \
    -machine virt -cpu cortex-a72 -m 512M \
    -global virtio-mmio.force-legacy=false \
    -drive if=none,file=disk.img,format=raw,id=hd \
    -device virtio-blk-device,drive=hd \
    -netdev dgram,id=net0,local.type=inet,local.host=127.0.0.1,local.port=23000,remote.type=inet,remote.host=127.0.0.1,remote.port=23001 \
    -device virtio-net-device,netdev=net0,mac=52:54:00:12:34:56 \
    -fw_cfg name=opt/veil.mode,string=net \
    -display none -serial "file:$S1" \
    -no-reboot -semihosting \
    -kernel "$KERNEL" &
Q1=$!
wait "$PEER"
PEER_RC=$?
kill "$Q1" 2>/dev/null
wait "$Q1" 2>/dev/null
cat /tmp/veil-netpeer.log
grep -c FAIL /tmp/veil-netpeer.log >/dev/null && true
result "netpeer checks all green" "$PEER_RC"
for s in "NET_TX" "NET_RX" "M12_OK" "ARP: who-has" "ICMP_OK" "M13_OK" "UDP: echoed"; do
    grep -q "$s" "$S1"; result "phase-1 serial '$s'" $?
done

# ---------------------------------------------------------------------------
note "phase 2: the Mac's own nc / curl / browser over slirp, with pcap"
S2=/tmp/veil-net2-serial.log
PCAP=shots/net.pcap
rm -f "$S2" "$PCAP"
qemu-system-aarch64 \
    -machine virt -cpu cortex-a72 -m 512M \
    -global virtio-mmio.force-legacy=false \
    -drive if=none,file=disk.img,format=raw,id=hd \
    -device virtio-blk-device,drive=hd \
    -netdev user,id=net0,hostfwd=tcp:127.0.0.1:7707-:7777,hostfwd=tcp:127.0.0.1:8080-:80 \
    -device virtio-net-device,netdev=net0 \
    -object filter-dump,id=fd0,netdev=net0,file="$PCAP" \
    -fw_cfg name=opt/veil.mode,string=net \
    -display none -serial "file:$S2" \
    -no-reboot -semihosting \
    -kernel "$KERNEL" &
Q2=$!
wait_serial "$S2" "SRV:" 60
result "guest serving (SRV sentinel)" $?

echo "--- M14: nc (the Mac's own TCP client) against the echo service"
NC_OUT=$(printf 'hello from the mac\n' | nc -w 5 127.0.0.1 7707)
echo "$NC_OUT"
echo "$NC_OUT" | grep -q "VEIL TCP ECHO"; result "nc got the greeting (guest->mac bytes)" $?
echo "$NC_OUT" | grep -q "echo: hello from the mac"; result "nc got its line echoed (mac->guest->mac)" $?
wait_serial "$S2" "M14_OK" 10
result "serial M14_OK (clean two-way close observed by the stack)" $?

echo "--- M15: curl fetches the site served off the FAT16 disk"
H=/tmp/veil-curl-headers
curl -s -m 10 -D "$H" -o /tmp/veil-index.htm http://127.0.0.1:8080/
cmp -s /tmp/veil-index.htm site/index.htm; result "GET / == site/index.htm byte-for-byte" $?
grep -qi "Content-Type: text/html" "$H"; result "index content-type text/html" $?
curl -s -m 10 -D "$H" -o /tmp/veil-style.css http://127.0.0.1:8080/style.css
cmp -s /tmp/veil-style.css site/style.css; result "GET /style.css matches" $?
grep -qi "Content-Type: text/css" "$H"; result "css content-type" $?
curl -s -m 10 -D "$H" -o /tmp/veil-logo.png http://127.0.0.1:8080/logo.png
cmp -s /tmp/veil-logo.png site/logo.png; result "GET /logo.png matches (binary)" $?
grep -qi "Content-Type: image/png" "$H"; result "png content-type" $?
curl -s -m 10 -o /tmp/veil-page2.htm http://127.0.0.1:8080/page2.htm
cmp -s /tmp/veil-page2.htm site/page2.htm; result "GET /page2.htm matches" $?
CODE=$(curl -s -m 10 -o /dev/null -w '%{http_code}' http://127.0.0.1:8080/nope.htm)
[ "$CODE" = "404" ]; result "missing file -> 404 (got $CODE)" $?
wait_serial "$S2" "M15_OK" 5
result "serial M15_OK" $?

echo "--- M15: the Mac's real default browser loads the page"
if open -g "http://127.0.0.1:8080/" 2>/dev/null; then
    # A real browser parses the HTML and fetches the subresources itself —
    # curl never asked for these. Seeing them in serial proves a browser.
    wait_serial "$S2" "GET /style.css" 45 && wait_serial "$S2" "GET /logo.png" 45
    result "browser fetched page + subresources (style.css, logo.png)" $?
else
    echo "WARN: 'open' failed (no GUI session?); browser check skipped"
    FAILED=1
fi

sleep 1
kill "$Q2" 2>/dev/null
wait "$Q2" 2>/dev/null
echo "--- phase 2 serial ---------------------------------------------"
cat "$S2"
echo "----------------------------------------------------------------"

note "packet capture: handshake + teardown verification"
python3 scripts/checkpcap.py "$PCAP" 7777
result "pcap handshake/teardown checks" $?
echo "--- tcpdump view of the nc session (first 25 lines)"
tcpdump -nr "$PCAP" "tcp port 7777" 2>/dev/null | head -25

note "verdict"
if [ "$FAILED" -eq 0 ]; then
    echo "ALL NET CHECKS PASSED (M12, M13 protocol-level, M14, M15)"
else
    echo "FAILURES PRESENT"
fi
exit "$FAILED"
