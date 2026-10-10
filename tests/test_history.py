"""Full history (0.10): messages past the recent limit move to data/history
instead of being dropped, load a page at a time, keep their files, can be
deleted, survive a restart; admins are warned when files near the limit."""
import asyncio, os

from lib import Server, check, main


async def drain(conn, kind):
    while await conn.wait(kind, timeout=0.3):
        pass


async def older(conn, before=None):
    msg = {"type": "loadOlder", "channel": "general"}
    if before is not None:
        msg["before"] = before
    await conn.send(**msg)
    return await conn.wait("olderMessages")


async def post_file(s, conn, key, name, size):
    _, r = s.upload(key, name, os.urandom(size))
    await conn.send(type="post", channel="general", text=name,
                    files=[{"upload": r["id"], "width": 0, "height": 0}])
    return r["id"]


async def run():
    s = Server(HISTORY_LIMIT=10, MAX_ATTACHMENT_MB=1, MAX_STORAGE_MB=1)
    alice, w = await s.register("Alice")
    check("olderMessages" in w.get("features", []), "the server says it keeps older messages")
    s.console("admin Alice")
    await alice.wait("accountUpdated")
    bob, wb = await s.register("Bob")
    key = wb["fileKey"]

    first = await post_file(s, bob, key, "first.bin", 200_000)
    await bob.wait("chat")
    for i in range(18):  # two senders: at most 10 messages each per 5 seconds
        sender = bob if i % 2 else alice
        await sender.send(type="chat", channel="general", text=f"m{i}")
        while (m := await sender.wait("chat")) and m["message"]["text"] != f"m{i}":
            pass  # wait for this one before sending the next, to keep the order
    await drain(alice, "chat")
    check(await alice.wait("storageWarning", timeout=0.3) is None, "no storage warning while there's room")

    carl, wc = await s.register("Carl")
    recent = [m["text"] for m in wc["history"]["general"]]
    check(recent == [f"m{i}" for i in range(8, 18)], "sign-in still sends only the recent messages")
    o = await older(carl)
    texts = [m["text"] for m in o["messages"]] if o else []
    check(texts == ["first.bin"] + [f"m{i}" for i in range(8)], "older messages load, oldest first")
    check(o and o["start"] == 0, "...and the server says there's nothing older")
    check(s.http("GET", f"/files/{first}", key)[0] == 200, "files of older messages stay downloadable")
    check(os.path.exists(f"{s.dir}/data/history/general.jsonl"), "older messages are in data/history")

    m3 = next(m["id"] for m in o["messages"] if m["text"] == "m3")
    await alice.send(type="deleteMessage", channel="general", id=m3)
    d = await carl.wait("messageDeleted")
    check(d and d["id"] == m3, "admins can delete an older message")
    o = await older(carl)
    check(o and "m3" not in [m["text"] for m in o["messages"]], "...and it's gone from the older pages")

    # Warnings for admins, and the storage limit with older files.
    await post_file(s, bob, key, "big.bin", 650_000)
    warn = await alice.wait("storageWarning")
    check(warn and "limit" in warn["text"], "admins are warned past 80% of the storage limit")
    check(await carl.wait("storageWarning", timeout=0.5) is None, "...others aren't")
    await post_file(s, bob, key, "more.bin", 300_000)
    gone = await alice.wait("attachmentGone")
    check(gone and gone["id"] == first, "past the limit, the oldest file goes first, even an older message's")
    o = await older(carl)
    f = next((m for m in o["messages"] if m["text"] == "first.bin"), None) if o else None
    check(f and f["attachments"][0].get("expired"), "...and shows as expired in the older pages")

    await asyncio.sleep(1.5)  # recent messages are saved once a second
    s.restart()
    alice, _ = await s.login("Alice")
    warn = await alice.wait("storageWarning")
    check(warn is not None, "admins get the warning when they sign in")
    o = await older(alice)
    texts = [m["text"] for m in o["messages"]] if o else []
    check("first.bin" in texts and "m0" in texts and "m3" not in texts,
          "after a restart older messages are still there (the deleted one isn't)")
    check(len(texts) == len(set(m["id"] for m in o["messages"])), "...each once")
    s.stop()


main(run)
