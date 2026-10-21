# Sync a note to Anytype

Humla can push a note to your [Anytype](https://anytype.io) knowledge base as
one object: the Title becomes the object's name, and the notes, summary and
transcript become its body, as Markdown. A note is pushed only when you push
it. Nothing syncs in the background.

The sync is **one way** (see `docs/adr/0007-anytype-sync-is-one-way.md`):
Humla creates and updates objects in Anytype, and never reads anything back.
An edit you make to a synced object in Anytype is **replaced the next time you
sync that note**. Humla has no conflict resolution to offer — the note is the
source of truth.

## Setting it up

1. In Anytype, open **Settings → API Keys** and click **Create New**. Name it
   "Humla" so you remember what it is for.
2. In Humla, open **Settings → General → Integrations**, turn on **Sync notes
   to Anytype**, paste the key, and click **Test connection**.
3. Pick the **space** the notes go to, and the **object type** they become
   (`Page` is the safe default). Click **Save Anytype settings**.

- The API key is kept in the macOS Keychain on this Mac. It is not synced and
  not stored in Humla's database. A saved key is never shown again; leave the
  field empty to keep it.
- Humla talks to Anytype's local API, which runs inside the Anytype app on this
  Mac. The address is fixed to `127.0.0.1` (configurable, but loopback only) —
  a note cannot leave the Mac through this path.
- The space and object type are stored in Humla's database; the key is not.

## Syncing a note

Open the note's **⋯** menu and choose **Sync to Anytype…**. The first push of
a note asks for confirmation, naming the space, and offers **Don't ask again
for this note** — tick it and later pushes are one click. **Send** pushes the
note; **Cancel**, Escape or a click outside pushes nothing. Your latest edits
are saved first, so Anytype gets what is on screen.

- **Sync again to update.** Each push rewrites the object's name and whole
  body. A push that finds nothing changed does nothing.
- **What the object gets.** A details list (when it was recorded, how long, how
  many takes, the Folder, the Client, who spoke), then the Summary, Transcript
  and Notes sections — the same content the Markdown export writes, so the two
  never disagree. An empty section reads as *(empty)*.
- **Audio is never sent**, and neither is any path to an audio file.
- **Sending during a recording** syncs the transcript so far, with
  *recording in progress* in the details. Sync again once the recording is
  processed.
- A note in the Trash can't be synced. Restore it first.

## Sending every note

**Send all notes to Anytype** in the same Settings section pushes every live
note, one object each, paced to stay inside Anytype's rate limit (~1 change per
second — a large library takes a while). Progress shows as *Syncing N of M…*
with a **Stop**. Notes already up to date are skipped.

## When a push fails

The error says what to do:

- **Anytype isn't running**: open Anytype and try again. Notes stay linked and
  are simply pushed the next time.
- **The API key was rejected**: create a new key in Anytype → Settings → API
  Keys and save it in Humla's Settings.
- **Rate limited**: wait a moment; a bulk run is paced, so this is rare.
- **The object was deleted or archived in Anytype**: the next push creates a
  fresh object and tells you it did.

No error message ever includes the API key or the note's content.

## Deleting things

- **Deleting a note in Humla** (moving it to the Trash) does nothing in
  Anytype. **Purging** it from the Trash forgets the mapping; the object stays
  in Anytype. Deleting it there is yours to do.
- **Changing the space** in Settings doesn't move anything: notes already
  synced stay in the old space, and are sent to the new space the next time
  each is synced. Humla never deletes an object it didn't create in this
  session — and never deletes one automatically at all.
