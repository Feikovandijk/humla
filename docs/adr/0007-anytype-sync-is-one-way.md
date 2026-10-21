# Anytype sync is one-way: Humla pushes, never pulls

Humla can push a Note to the user's Anytype knowledge base as one object
(title = the Note's Title, body = its notes, summary and transcript as
Markdown). We decided the sync is **one-way and Humla is the source of
truth**: Humla creates and updates objects in Anytype and never reads anything
back. An edit made to a synced object in Anytype is replaced the next time the
note is synced.

## Considered Options

- **Two-way sync with last-write-wins.** Rejected. ADR-0004 makes the Timeline
  canonical for a note's content and `note.transcript` a projection of it;
  text pulled back from Anytype would have to enter through a path that writes
  the transcript string directly — the exact defect ADR-0004 closed. Mapping
  Anytype blocks back onto timeline entries needs a per-block identity the
  Markdown body push doesn't establish, and "the user edited both sides" needs
  a conflict story (merge? pick a winner? show both?) that a meeting-notes app
  has no answer for and no user has asked for.
- **One-way with a warning (chosen).** The first push of a note is an explicit,
  confirmed act that names the space and states the overwrite rule; updates
  after that are one click. The object in Anytype is a *copy for reading and
  linking*, not an editable mirror. Humla stores the mapping (`anytype_links`:
  note id → object id, plus a fingerprint of what was last sent) so a re-push
  updates the same object instead of duplicating it.
- **Export-only (no live sync).** Rejected as the whole feature: the point of
  Anytype is that the note lands in the graph where it can be queried, linked
  and found. The webhook (#213) already covers "hand it to a script once";
  sync covers "keep my knowledge base current".

## Consequences

- **The document is a rendering, not a record.** What Anytype holds is derived
  from the Note at push time, so the push can be rebuilt any time from the
  database — no local copy of the sent document is kept, only its hash.
- **The confirmation is per note and sticky.** It asks until the user ticks
  "Don't ask again for this note", like the webhook asks every time but with
  an opt-out. A bulk push is explicit consent for every note it covers.
- **Deletion is never automatic.** Trashing or purging a note never deletes the
  Anytype object; purging forgets the mapping. Changing the space leaves old
  objects in the old space and sends notes to the new one on their next push.
- **The destination is loopback.** Anytype's API runs inside the Anytype app on
  the same Mac; the base URL is restricted to `127.0.0.1` / `localhost` /
  `::1`, so a push cannot leave the machine through a configured endpoint.
- **The API key is a Keychain secret**, write-only from the webview, exactly
  like the webhook endpoint (#213).
- **Audio stays out.** `keep_audio` remains the only gate on audio; the sync
  carries text only, and no path to any audio file.
- **Known cost: an Anytype edit is silently lost on the next sync.** The
  modal, the Settings row and `docs/anytype.md` all say so. If two-way ever
  becomes a requirement, this ADR is the document to revisit — and the block
  identity problem above is the work.
