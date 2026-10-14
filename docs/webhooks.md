# Send a note to a webhook

Humla can POST a note to a URL you choose, so your own tools (a knowledge
base, an agent, an n8n or Zapier flow) can receive the meeting. A note is sent
only when you send it. Nothing goes out in the background.

## Setting it up

In **Settings → General → Integrations**, turn on **Send notes to a webhook**,
enter the receiver's URL, add any headers it needs for authentication (for
example `Authorization` with a value of `Bearer <token>`), and click **Save
webhook**.

- The URL and headers are kept in the macOS Keychain on this Mac. They are not
  synced and not stored in Humla's database.
- A saved header's value is never shown again. Leave its field empty to keep
  it, or type a new one to replace it. Renaming a header needs its value
  re-entered.
- Use HTTPS for anything that isn't on your own machine. HTTP is accepted for
  local receivers.
- Put authentication in a header, not in the URL. A URL with a username or
  password, or with a `#fragment`, is refused. A token in the query string
  works but is less private: it travels in the URL.

## Sending a note

Open the note's **⋯** menu and choose **Send to webhook…**. The confirmation
names the receiver's host (never its path or query, which often hold a
token). **Send** posts the note; **Cancel**, Escape or a click outside sends
nothing. Your latest edits are saved first, so the receiver gets what is on
screen.

- **Send again to update.** Each send is a full snapshot of the note. Your
  receiver should upsert by `note.id`.
- **No retries, no redirects.** A send is one request with a 10 s connect
  timeout and a 30 s overall timeout. A redirect is treated as a failure: put
  the final URL in Settings.
- **Success means the receiver accepted it.** Any 2xx reads as sent, which
  says nothing about what your workflow did with it afterwards.
- **A timeout is uncertain.** The receiver may still have got the note. Check
  before sending again.
- **Sending during a recording** sends the transcript so far, with
  `recording.in_progress` set. Send again once the recording is processed.
- There is no event for a deleted note. A note in the Trash can't be sent.

The request is `POST` with `Content-Type: application/json` and
`User-Agent: Humla/<version>`, plus your headers.

## Payload (schema version 1)

```json
{
  "schema_version": 1,
  "event": "note.sent",
  "sent_at": "2026-10-12T09:45:00.000Z",
  "note": {
    "id": "5f0c3a52-8a8e-4a0e-9a53-0b8c1f3e6c21",
    "title": "Weekly planning",
    "created_at": "2026-10-12T08:58:12.000Z",
    "updated_at": "2026-10-12T09:44:03.000Z",
    "language": "en",
    "folder": { "id": "1d7e…", "name": "Product" },
    "client": null,
    "notes_html": "<p>Confirm the launch date.</p>",
    "notes_text": "Confirm the launch date.",
    "summary": "- Launch confirmed for the 24th.",
    "transcript": "You: Are we ready?\nAlex: Yes, the 24th.",
    "speakers": ["You", "Alex"],
    "recording": {
      "started_at": "2026-10-12T09:00:00.000Z",
      "duration_ms": 1800000,
      "in_progress": false,
      "takes": [
        { "index": 1, "started_at": "2026-10-12T09:00:00.000Z", "duration_ms": 1800000 }
      ]
    }
  }
}
```

Fields may be added to schema version 1. Existing fields keep their meaning,
so ignore any field you don't know. A change that breaks this contract gets a
new `schema_version`.

| Field | Meaning |
|---|---|
| `sent_at`, `created_at`, `updated_at` | RFC 3339 timestamps in UTC, with milliseconds. |
| `id` | The note's stable id. Upsert on it. |
| `language` | ISO 639-1 code of the note's language: the one set on the note, else Humla's default, with *Auto* resolved to what was detected. `null` when it is *Auto* and nothing was detected. |
| `folder`, `client` | `{ id, name }` of the folder or client the note is filed under, or `null`. The id survives a rename. |
| `notes_html` | What you typed in the note, as the editor's HTML. Older notes may hold plain text. |
| `notes_text` | The same as plain text. |
| `summary` | The summary as Markdown. Empty if none was made. |
| `transcript` | The transcript as plain text, one turn per line, each starting with its speaker's label (`Alex: …`) once speakers are identified. |
| `speakers` | Each label in `transcript` once, in order of first appearance, renamed speakers included. Labels are names, not person ids. |
| `recording` | When the note was recorded (below), or `null` for a note that never was. |

Audio is never sent, and neither is any path to an audio file, your headers,
or Humla's own settings.

### Recording timing

A note can be recorded in several takes. `recording.takes` lists them in
order, each with when it started and how long it ran. `started_at` is the
earliest start. `duration_ms` is the sum of the takes, without the gaps
between them, so it is not `end − start`. Durations are measured on the
recorded audio, so time spent paused isn't counted. A take's duration ends at
its last transcribed speech, or at the end of its audio when it has no
transcript yet.

A value that isn't known is `null`, never guessed from when the note was
created. That covers takes from before Humla recorded their timing. If any
take's start or duration is unknown, the matching total is `null` too, while
the takes that are known keep their values. For imported audio, a take's start
is when it was imported, not when the original meeting happened.

While the note is being recorded (or audio is being imported into it), the take
in progress isn't listed yet:
`in_progress` is `true`, `duration_ms` is `null`, and `takes` holds only the
finished ones.

## When a send fails

The error says what the receiver answered:

- **401 or 403**: check the headers in Settings.
- **413**: the note is larger than the receiver accepts. The message gives the
  payload's size. Raise the receiver's request body limit. Humla never
  truncates a note to fit.
- **3xx**: redirects aren't followed. Put the final URL in Settings.
- **Anything else**: check the receiver's logs.

No message ever includes the URL or anything the receiver sent back, since
either could carry a token or the meeting itself.
