# Send a meeting to a webhook

In **Settings → General → Webhook**, enable the webhook, enter the receiver URL,
add any authentication headers, and click **Save webhook**. The URL and headers
are kept in macOS Keychain on this device; they are not synced to teammates.
Header values are masked in Settings. Use HTTPS for remote receivers; HTTP is
also supported for local services.

Click **Send to webhook** in a note's toolbar, then **Yes** in the confirmation
dialog to POST its current contents as `application/json`. **No**, Escape, or
clicking outside the dialog cancels without sending. The button is hidden until
enabled. Each confirmation sends one
snapshot, including edits still waiting for autosave. Sending during a recording
sends the transcript available at that moment; send again after processing to
update your receiver with the finished meeting.

The receiver should upsert by `note.id` to support sending the same note again.
There is no background sync, deletion event, or automatic retry. A successful
2xx response is shown as sent; this confirms the receiver accepted the request,
not that any downstream workflow finished. Requests time out after 30 seconds.
A timeout/network error can leave delivery uncertain, so check the receiver
before sending again. Redirects are rejected: configure the final endpoint URL.

## Payload (schema version 1)

```json
{
  "schema_version": 1,
  "event": "note.sent",
  "sent_at": "2026-10-10T12:00:00+00:00",
  "note": {
    "id": "note-uuid",
    "title": "Weekly planning",
    "body": "<p>Confirm the launch date.</p>",
    "body_text": "Confirm the launch date.",
    "transcript": "You: Ready?\nAlex: Yes.",
    "summary": "- Launch confirmed.",
    "speakers": ["You", "Alex"],
    "language": "en",
    "detected_language": null,
    "summary_preset": "meeting",
    "summary_provider": "",
    "expected_speakers": null,
    "folder_id": null,
    "client_id": null,
    "workspace_id": "",
    "owner": "",
    "private": false,
    "created_at": 1791633600000,
    "updated_at": 1791633600000,
    "meeting": {
      "started_at": "2026-10-10T09:00:00+00:00",
      "ended_at": "2026-10-10T09:30:00+00:00",
      "duration_ms": 1800000,
      "end_time_estimated": true,
      "recording_in_progress": false,
      "sessions": [
        {
          "id": "recording-session-uuid",
          "index": 1,
          "started_at": "2026-10-10T09:00:00+00:00",
          "ended_at": "2026-10-10T09:30:00+00:00",
          "duration_ms": 1800000,
          "streams": ["mic", "sys"]
        }
      ]
    }
  }
}
```

`body` preserves the editor's HTML (legacy notes may be plain text);
`body_text` is its plain-text equivalent. `summary` is Markdown. The full
`transcript` is one plain-text string, preserved verbatim with speaker labels
and line breaks (for example, `Johannes: Hello\nFredrik: Hi`). JSON encodes
line breaks as `\n`; decoding the JSON restores real line breaks. There is no
duplicate structured `turns` array. `speakers` lists unique current labels in
first-appearance order, including renamed speakers. Labels are names, not
persistent person IDs.

`created_at` and `updated_at` are the stored Unix timestamps in milliseconds;
`sent_at` is the snapshot's delivery timestamp, including any unsaved edits.
Empty language/provider values mean the note follows the global setting.
Audio, local file paths, auth headers, and unrelated settings are never included.

### Meeting timing

`note.meeting` describes persisted recording sessions. `started_at` is the
earliest session start; `ended_at` is the latest estimated session end (start
plus stored duration). These are RFC3339 timestamps. `duration_ms` sums the
session durations, without gaps between separate recordings or double-counting
microphone and system audio. It is not necessarily `ended_at - started_at`.
Stored durations are best-effort capture/timeline lengths. End times are
estimates, not independently recorded stop timestamps; pauses, missing trailing
audio, and imported audio can differ from the real meeting's wall-clock bounds.
For imports, the stored start is the import time, not the original event time.

Unknown timing (including older recordings) is `null`, never guessed from note
creation or modification times. If any session lacks a date/duration, the
corresponding overall value is `null`; individual known sessions remain
available. While recording, the overall end and duration stay `null` and
`recording_in_progress` is true. The sessions array contains finalized sessions
only; send again once processing finishes for complete timing.

### Delivery diagnostics and HTTP 413

HTTP 413 means the receiver or a proxy rejected the request as too large. Humla
reports the exact serialized JSON byte count; it does not truncate the transcript
or retry automatically. The transcript is included once, as plain text.

The most recent attempt is recorded in `diagnostics/webhook-last.json` inside
the app's data directory. It contains the attempt time, payload byte count,
successful HTTP status or sanitized error. It contains no webhook URL, auth
headers, note content, or note identifier. Older builds did not retain these
diagnostics, so they cannot recover a historical failed request.
