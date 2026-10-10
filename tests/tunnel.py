"""A stand-in for Cloudflare's tunnel: HTTPS in front, plain HTTP to the server,
request bodies re-sent with Transfer-Encoding: chunked, CF-Connecting-IP added."""
import asyncio, ssl, sys

LISTEN, ORIGIN = int(sys.argv[1]), int(sys.argv[2])
D = sys.argv[3]


async def pipe(r, w):
    try:
        while data := await r.read(65536):
            w.write(data)
            await w.drain()
    except Exception:
        pass
    finally:
        try:
            w.close()
        except Exception:
            pass


async def handle(cr, cw):
    try:
        head = await cr.readuntil(b"\r\n\r\n")
    except Exception:
        cw.close()
        return
    lines = head.decode("latin1").split("\r\n")
    method = lines[0].split(" ")[0]
    headers = [l for l in lines[1:] if l]
    h = {l.split(":", 1)[0].strip().lower(): l.split(":", 1)[1].strip() for l in headers}
    orr, ow = await asyncio.open_connection("127.0.0.1", ORIGIN)
    keep = [l for l in headers if l.split(":", 1)[0].strip().lower() not in ("content-length", "transfer-encoding")]
    keep.append("CF-Connecting-IP: 198.51.100.9")
    if "websocket" in h.get("upgrade", "").lower():
        ow.write((lines[0] + "\r\n" + "\r\n".join(keep) + "\r\n\r\n").encode("latin1"))
        await ow.drain()
        await asyncio.gather(pipe(cr, ow), pipe(orr, cw))
        return
    if len(sys.argv) > 4 and sys.argv[4] == "break-http":
        cw.write(b"HTTP/1.1 502 Bad Gateway\r\nContent-Length: 0\r\nConnection: close\r\n\r\n")
        await cw.drain()
        cw.close()
        return
    n = int(h.get("content-length", "0") or 0)
    body = await cr.readexactly(n) if n else b""
    out = lines[0] + "\r\n" + "\r\n".join(keep) + "\r\n"
    if method in ("POST", "PUT"):
        out += "Transfer-Encoding: chunked\r\n\r\n"
        data = out.encode("latin1")
        for i in range(0, len(body), 65536):
            part = body[i:i + 65536]
            data += f"{len(part):x}\r\n".encode() + part + b"\r\n"
        data += b"0\r\n\r\n"
    else:
        data = (out + "\r\n").encode("latin1")
    ow.write(data)
    await ow.drain()
    await pipe(orr, cw)


async def main():
    ctx = ssl.create_default_context(ssl.Purpose.CLIENT_AUTH)
    ctx.load_cert_chain(f"{D}/chain.crt", f"{D}/srv.key")
    server = await asyncio.start_server(handle, "127.0.0.1", LISTEN, ssl=ctx)
    print("tunnel ready", flush=True)
    await server.serve_forever()


asyncio.run(main())
