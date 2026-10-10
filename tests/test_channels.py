"""Admins adding channels: only admins, names are cleaned, duplicates refused,
everyone gets the new lists, and they're still there after a restart."""
from lib import Server, check, main


async def run():
    s = Server()
    alice, w = await s.register("Alice")
    check("channels" in w.get("features", []), "the server says it can add channels")
    s.console("admin Alice")
    await alice.wait("accountUpdated")
    bob, _ = await s.register("Bob")

    await bob.send(type="createChannel", name="nope")
    e = await bob.wait("error")
    check(e and e["code"] == "channel_failed", "non-admins are refused")
    check(await alice.wait("channels", timeout=0.7) is None, "...and nothing is added")

    await alice.send(type="createChannel", name="  Movie Night! ")
    ca, cb = await alice.wait("channels"), await bob.wait("channels")
    check(ca and ca["textChannels"][-1] == "movie-night", "text channel names are cleaned")
    check(cb and cb["textChannels"] == ca["textChannels"], "everyone gets the new list")

    await alice.send(type="createChannel", name="Movie-night")
    e = await alice.wait("error")
    check(e and e["code"] == "channel_failed", "a name that's taken is refused")

    await alice.send(type="createChannel", name="Late Talk", voice=True)
    c = await bob.wait("channels")
    check(c and c["voiceChannels"][-1] == "Late Talk", "voice channels keep their name")
    await bob.send(type="joinVoice", channel="Late Talk")
    j = await bob.wait("voiceJoined")
    check(j and j["channel"] == "Late Talk", "the new voice channel can be joined")
    await alice.send(type="chat", channel="movie-night", text="hi")
    m = await bob.wait("chat")
    check(m and m["message"]["channel"] == "movie-night", "the new text channel works")

    s.restart()
    _, w = await s.register("Carl")
    check("movie-night" in w["textChannels"] and "Late Talk" in w["voiceChannels"],
          "new channels are still there after a restart")
    s.stop()
    check("added the text channel #movie-night" in s.log(), "the server log says who added it")


main(run)
