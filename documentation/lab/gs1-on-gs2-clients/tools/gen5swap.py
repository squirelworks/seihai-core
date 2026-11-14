#!/usr/bin/env python3
"""Lab-only gen-5 proxy that swaps NPC bytecode (seihai-core#6 lab).

listen LISTEN_HOST:PORT, forward to UP_HOST:UP_PORT. Both directions are decrypted and
logged like gen5log.py. Server->client bundles are re-encrypted with the proxy's own
cipher state; when a bundle holds PLO_RAWDATA(100) + PLO_NPCBYTECODE(131), the bytecode
after the GINT3 npc id is replaced with the bytes of PAYLOAD (read fresh each time) and
the RAWDATA length is fixed. Every swap is logged with the sha256 of the bytes sent.
Usage: gen5swap.py LISTEN_HOST PORT UP_HOST UP_PORT LOGFILE LABEL PAYLOAD
"""
import bz2, hashlib, os, select, socket, sys, threading, time, zlib


def crypt(data, key, state, typ):
    out = bytearray(data)
    groups = min((len(data) + 3) // 4, 12) if typ == 2 else min(len(data) // 4, 4)
    for g in range(groups):
        state = (state * 0x08088405 + key) & 0xffffffff
        m = state.to_bytes(4, 'little'); p = 4 * g
        for j in range(min(4, len(data) - p)):
            out[p + j] ^= m[j]
    return bytes(out), state


def show(b, n=160):
    s = ''.join(chr(c) if 32 <= c < 127 else '.' for c in b[:n])
    return s + ('...' if len(b) > n else '')


def gint3(b):
    return ((b[0] - 32) << 14) | ((b[1] - 32) << 7) | (b[2] - 32)


def mkgint3(n):
    return bytes([((n >> 14) & 0x7f) + 32, ((n >> 7) & 0x7f) + 32, (n & 0x7f) + 32])


class Dir:
    def __init__(self, name):
        self.name = name; self.buf = b''; self.state = 0x04a80b38; self.ostate = 0x04a80b38
        self.raw = None; self.stream = b''
        self.partial = b''; self.raw_left = 0  # rewrite parser


def packets(d, data, rec):
    d.stream += data
    while d.stream:
        if d.raw is not None:
            if len(d.stream) < d.raw:
                return
            p, d.stream = d.stream[:d.raw], d.stream[d.raw:]; d.raw = None
            if p.endswith(b'\n'):
                p = p[:-1]
        else:
            i = d.stream.find(b'\n')
            if i < 0:
                return
            p, d.stream = d.stream[:i], d.stream[i + 1:]
        if not p:
            continue
        pid = p[0] - 32; body = p[1:]; extra = ''
        if d.name == 'S>C' and pid == 100 and len(body) >= 3:
            d.raw = gint3(body); extra = 'rawlen=%d' % d.raw
        if d.name == 'S>C' and pid == 102 and len(body) > 6:
            nl = body[5] - 32; extra = 'FILE name=%s size=%d' % (body[6:6 + nl].decode('latin1'), len(body) - 6 - nl); body = body[:6 + nl]
        if d.name == 'S>C' and pid in (30, 45):
            extra = 'FILE-%s %s' % ('FAILED' if pid == 30 else 'UPTODATE', show(body))
        if d.name == 'S>C' and pid == 131 and len(body) >= 3:
            extra = 'NPCBYTECODE npc=%d len=%d sha256=%s' % (gint3(body), len(body) - 3, hashlib.sha256(body[3:]).hexdigest())
        if d.name == 'C>S' and pid == 23:
            extra = 'WANTFILE %s' % show(body)
        rec(d.name, 'id=%d' % pid, 'len=%d' % len(p), extra, '|', show(body), '| hex=' + body[:48].hex())


def rewrite(d, data, payload_path, rec):
    """Walk one decompressed S>C bundle; swap any complete 100+131 pair inside it."""
    out = bytearray(); i = 0; n = len(data)
    while i < n:
        if d.raw_left:
            take = min(d.raw_left, n - i); out += data[i:i + take]; d.raw_left -= take; i += take; continue
        j = data.find(b'\n', i)
        if j < 0:
            d.partial += data[i:]; out += data[i:]; break
        whole_here = not d.partial
        line = d.partial + data[i:j]; seg = data[i:j + 1]; d.partial = b''; i = j + 1
        if line and line[0] - 32 == 100 and len(line) >= 4:
            ln = gint3(line[1:4])
            if whole_here and i + ln <= n and ln >= 4 and data[i] - 32 == 131:
                raw = data[i:i + ln]; i += ln
                nl = raw.endswith(b'\n')
                body = raw[:-1] if nl else raw
                npc = gint3(body[1:4]); old = body[4:]
                try:
                    new = open(payload_path, 'rb').read()
                except OSError as e:
                    rec('SWAP-ERROR', repr(e)); new = old
                newraw = body[:4] + new + (b'\n' if nl else b'')
                out += line[:1] + mkgint3(len(newraw)) + line[4:] + b'\n' + newraw
                rec('SWAP npc=%d old_len=%d old_sha256=%s new_len=%d new_sha256=%s payload=%s' % (
                    npc, len(old), hashlib.sha256(old).hexdigest(), len(new), hashlib.sha256(new).hexdigest(), os.path.basename(payload_path)))
                continue
            if line[1:2] and data[i:i + 1] and data[i] - 32 == 131:
                rec('SWAP-MISS 131 spans bundles; forwarded unchanged')
            d.raw_left = ln
        out += seg
    return bytes(out)


def main():
    lh, lp, uh, up, logf, label, payload = sys.argv[1], int(sys.argv[2]), sys.argv[3], int(sys.argv[4]), sys.argv[5], sys.argv[6], sys.argv[7]
    log = open(logf, 'a', buffering=1); t0 = time.monotonic(); lock = threading.Lock()

    def rec(*a):
        with lock:
            log.write('%8.3f %s %s\n' % (time.monotonic() - t0, label, ' '.join(str(x) for x in a)))
    L = socket.socket(); L.setsockopt(socket.SOL_SOCKET, socket.SO_REUSEADDR, 1); L.bind((lh, lp)); L.listen(4)
    rec('listening', lh, lp, '->', uh, up, 'payload', payload)
    k = 0
    while True:
        c, _ = L.accept(); k += 1
        threading.Thread(target=handle, args=(c, uh, up, rec, k, payload), daemon=True).start()


def handle(c, uh, up, rec, n, payload):
    u = socket.create_connection((uh, up)); u.settimeout(None); rec('conn', n, 'open')
    cs, sc = Dir('C>S'), Dir('S>C'); key = None

    def frames_cs(data):
        cs.buf += data
        while len(cs.buf) >= 2:
            ln = int.from_bytes(cs.buf[:2], 'big')
            if len(cs.buf) < ln + 2:
                return
            fr, cs.buf = cs.buf[2:ln + 2], cs.buf[ln + 2:]
            typ = fr[0]; dec, cs.state = crypt(fr[1:], key, cs.state, typ)
            try:
                body = dec if typ == 2 else zlib.decompress(dec) if typ == 4 else bz2.decompress(dec) if typ == 6 else None
            except Exception as e:
                rec('C>S frame-decode-error', typ, e); continue
            if body is not None:
                packets(cs, body, rec)

    def frames_sc(data):
        """returns bytes to send to the client"""
        sc.buf += data; out = b''
        while len(sc.buf) >= 2:
            ln = int.from_bytes(sc.buf[:2], 'big')
            if len(sc.buf) < ln + 2:
                break
            fr, sc.buf = sc.buf[2:ln + 2], sc.buf[ln + 2:]
            typ = fr[0]; dec, sc.state = crypt(fr[1:], key, sc.state, typ)
            try:
                body = dec if typ == 2 else zlib.decompress(dec) if typ == 4 else bz2.decompress(dec) if typ == 6 else None
            except Exception as e:
                rec('S>C frame-decode-error', typ, e); body = None
            if body is None:  # cannot touch it: forward as is (cipher states stay in step only if sizes match)
                enc, sc.ostate = crypt(dec, key, sc.ostate, typ)
                out += (len(enc) + 1).to_bytes(2, 'big') + bytes([typ]) + enc; continue
            nb = rewrite(sc, body, payload, rec)
            packets(sc, nb, rec)
            if nb == body:
                comp = dec
            else:
                comp = nb if typ == 2 else zlib.compress(nb, 9) if typ == 4 else bz2.compress(nb)
                if len(comp) + 1 > 0xffff:
                    rec('SWAP-ERROR bundle too large'); comp = dec
            enc, sc.ostate = crypt(comp, key, sc.ostate, typ)
            out += (len(enc) + 1).to_bytes(2, 'big') + bytes([typ]) + enc
        return out

    try:
        while True:
            r, _, _ = select.select([c, u], [], [], 1)
            for s in r:
                data = s.recv(65536)
                if not data:
                    rec('conn', n, 'closed by', 'client' if s is c else 'server'); return
                if s is c:
                    u.sendall(data)
                    if key is None:
                        cs.buf += data
                        if len(cs.buf) >= 2 and len(cs.buf) >= int.from_bytes(cs.buf[:2], 'big') + 2:
                            ln = int.from_bytes(cs.buf[:2], 'big'); lg = zlib.decompress(cs.buf[2:ln + 2]); cs.buf = cs.buf[ln + 2:]
                            key = lg[1] - 32; ver = lg[2:10]; al = lg[10] - 32; acct = lg[11:11 + al]
                            rec('C>S LOGIN type=%d key=%d ver=%s account=%s password=<pw>' % (lg[0] - 32, key, ver.decode('latin1'), acct.decode('latin1')))
                            rest = cs.buf; cs.buf = b''; frames_cs(rest)
                    else:
                        frames_cs(data)
                else:
                    if key is None:
                        c.sendall(data)
                    else:
                        o = frames_sc(data)
                        if o:
                            c.sendall(o)
    except Exception as e:
        rec('conn', n, 'error', repr(e))
    finally:
        c.close(); u.close()


if __name__ == '__main__':
    main()
