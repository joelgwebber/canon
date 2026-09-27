---
id: canon-118a
title: A track that fails to open stops the queue instead of moving on
type: bug
priority: 2
created: '2026-09-27T23:42:49Z'
updated: '2026-09-27T23:42:49Z'
---

Seen with canon-e828 (2026-09-27): one unplayable entry left the player in Error, and nothing played until Joel pressed next. EngineEvent::Failed only sets state=Error. With autoplay/queue playback running, a failed open (not a failed output) probably should skip to the successor, with some bound so a queue of all-unplayable entries doesn't spin.
