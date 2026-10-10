"""Admins deleting messages: only admins, everyone sees it go, its files are
deleted, and it stays deleted after a restart."""
import asyncio, os

from lib import Server, check, main


async def run():
    s = Server()
    alice, _ = await s.register("Alice")
    s.console("admin Alice")
    await alice.wait("accountUpdated")
    bob, wb = await s.register("Bob")
    key = wb["fileKey"]
    _, fid = s.upload(key, "clip.mp4", b"\0\0\0\x20ftypisom" + os.urandom(50_000))
    _, pid = s.upload(key, "poster.jpg", b"\xff\xd8\xff\xe0" + os.urandom(5_000))
    fid, pid = fid["id"], pid["id"]
    await bob.send(type="post", channel="general", text="delete me",
                   files=[{"upload": fid, "width": 640, "height": 360, "durationMs": 1000, "poster": pid}])
    m = await alice.wait("chat")
    check(m and m["message"]["text"] == "delete me", "message arrives")
    mid = m["message"]["id"]
    await bob.send(type="chat", channel="general", text="keep me")
    keep = (await alice.wait("chat"))["message"]["id"]
    await bob.wait("chat"), await bob.wait("chat")
    check(s.http("GET", f"/files/{fid}", key)[0] == 200, "file downloadable before")

    await bob.send(type="deleteMessage", channel="general", id=mid)
    e = await bob.wait("error")
    check(e and e["code"] == "not_admin", "non-admins are refused")
    check(await alice.wait("messageDeleted", timeout=0.7) is None, "...and nothing is deleted")

    await alice.send(type="deleteMessage", channel="general", id=mid)
    da, db = await alice.wait("messageDeleted"), await bob.wait("messageDeleted")
    check(da and da["id"] == mid and da["channel"] == "general", "the admin sees it deleted")
    check(db and db["id"] == mid, "everyone else sees it deleted")
    check(s.http("GET", f"/files/{fid}", key)[0] == 404, "its file is deleted")
    check(s.http("GET", f"/files/{pid}", key)[0] == 404, "its poster is deleted")
    att = f"{s.dir}/data/attachments"
    left = [f for f in os.listdir(att) if fid in f or pid in f] if os.path.isdir(att) else []
    check(not left, "nothing left on disk")

    await alice.send(type="deleteMessage", channel="general", id=mid)
    check(await alice.wait("messageDeleted") is not None, "deleting it again is harmless")
    await alice.send(type="deleteMessage", channel="nope", id="x")
    await alice.wait("messageDeleted", timeout=0.5)

    await asyncio.sleep(1.5)  # history is saved once a second
    s.restart()
    _, w = await s.register("Carl")
    ids = [h["id"] for h in w["history"].get("general", [])]
    check(mid not in ids and keep in ids, "after a restart only the other message is there")
    s.stop()
    check("deleted a message by Bob" in s.log(), "the server log says who deleted whose message")


main(run)
