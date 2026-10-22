---
id: canon-ef76
title: Fail a lost renderer through SinkFailed, never a SelectSink(local)
status: shorn
type: bug
priority: 2
created: '2026-10-04T16:24:10Z'
updated: '2026-10-04T16:50:16Z'
parent: canon-5303
labels:
- sink
verify: cargo test -p canon-daemon reconnect && cargo test -p canon-core sink_failure
---

When reconnect() fails, select_sink has already dropped the session (same speaker) and its error branch sends Command::SelectSink(local): user intent, indistinguishable to the player from someone picking local, which breaks 'device status is authoritative'. reconnect's own fail_back_to_local then finds no session of its epoch and does nothing, so SinkFailed is never sent. The same-speaker error branch of a user's own switch (Tunes@cast to Tunes@dlna failing) has the same shape. Route both through player.sink_failed with the reason.

---
▸ 2026-10-04T16:50:16Z [Joel Webber]

verify: `cargo test -p canon-daemon reconnect && cargo test -p canon-core sink_failure` -> PASS (exit 0)

---
▸ 2026-10-04T16:50:16Z [Joel Webber]

Shorn 2026-10-04. reconnect() releases the dead session itself and, if select_sink fails, reports player.sink_failed(id, why) instead of relying on fail_back_to_local (a no-op once the session is gone). select_sink's same-speaker release, when the new connect fails, reports the released sink as lost via sink_failed instead of Command::SelectSink(local). On hardware (Library display, netfault): a refused reconnect ended with SinkFailed reaching the player ('lost output … wasn't back within 30s'); re-selecting Library with its :8009 ignored gave the client 'error: sink: Library display (192.168.0.82:8009) didn't answer within 8s' then 'lost output …: switching it to Chromecast failed: …'. Both are the player hearing a loss, not a user choosing local.
