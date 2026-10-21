# Anytype local API — what the sync integration rests on

Research for the Anytype sync (ADR 0007), gathered 2026-10-21 from Anytype's
own documentation: the Developer Portal (`developers.anytype.io`, API v1
reference version `2025-11-08`), the doc site's Local API page
(`doc.anytype.io/anytype/features/integrations/local-api`), the v2 agent skill
(`developers.anytype.io/skill.md`), and the raw OpenAPI spec
(`raw.githubusercontent.com/anyproto/anytype-api/main/docs/reference/openapi-2025-11-08.yaml`).

**Status: documentation-verified, not yet device-verified.** Everything below
is what the docs promise; the open questions at the end need one run against
a real Anytype install before release. The client tests pin the request shapes
against a stub server, so a doc/behaviour mismatch fails loudly in CI rather
than in front of a user.

## The shape of it

- **Local REST API served by the Anytype desktop app** at
  `http://127.0.0.1:31009` (shipped with the app since 0.46.x; the app must be
  running — a refused connection means "Anytype isn't open"). Fully offline.
- **Auth**: `Authorization: Bearer <key>`. Keys are created in the app
  (Vault Settings → Data Management → API Keys → Create New); the API mints
  none. A key may be scoped to spaces and to read-only.
- **Version header**: `Anytype-Version: 2025-11-08` (the current v1). v2
  (`/v2/...`, objects as editable JSON documents with PATCH ops) is
  **pre-release** — do not build on it.
- **Rate limit**: sustained ~1 request/second, burst 60 (disable-able only by
  an env var on the Anytype side). The sync queue paces itself at ~1.1 s
  between requests so a bulk run stays under it.

## Endpoints the integration uses

| Call | Request | Notes |
|---|---|---|
| List spaces | `GET /v1/spaces?limit=100` | `{ data: [{ id, name, … }] }` |
| List types | `GET /v1/spaces/{space}/types?limit=100` | `{ data: [{ key, name, … }] }`; built-in keys include `page`, `note` |
| Create object | `POST /v1/spaces/{space}/objects` | body `{ type_key, name, body }` — **the Markdown body goes in `body` on create** |
| Update object | `PATCH /v1/spaces/{space}/objects/{id}` | body `{ name, markdown }` — **and in `markdown` on update**; replaces the whole body, no partial patch in v1 |
| (available, unused) | `DELETE /v1/spaces/{space}/objects/{id}` | archives the object; deliberately never called automatically |

The create/update field asymmetry (`body` vs `markdown`) is real in the spec
and pinned by a client test: `create_sends_body_and_update_sends_markdown`.

- Create returns `201` with `{ object: { id, … } }`; update returns `200`.
- Errors are `{ status, code, message }` — the `message` is short and
  user-safe; Humla surfaces it bounded to 160 chars.
- `404`/`410` on update means the object is gone (deleted/archived) — the
  sync recreates it and says so.

## Decisions taken from these facts

- **Loopback-only base URL** (`127.0.0.1` / `localhost` / `::1`, no path, no
  credentials, no fragment): the API is local by design, and a configurable
  remote endpoint would let a meeting leave the Mac through a settings field.
  It also keeps the client tests possible — the stub server binds a random
  loopback port.
- **Whole-body replace on update** is fine for this feature because the
  document is rebuilt from the note on every push; nothing is merged, so
  Anytype-side edits are overwritten — stated in the modal, Settings and
  `docs/anytype.md` (ADR 0007).
- **Paced single worker** instead of concurrent pushes, because of the rate
  limit.
- **No properties/types/tags are written.** The object is a plain page (or a
  user-chosen type) with a Markdown body; Humla creates no schema in the
  user's space. (v1 supports property writes; a typed "Meeting" object is the
  natural phase 2.)

## Open questions for the device spike

Run once against a scratch space with a throwaway key before release:

1. **Markdown fidelity**: nested lists, GFM tables and task lists from
   summaries; a transcript with many turns (we paragraph-join turns because
   CommonMark folds single newlines — confirm Anytype's parser agrees).
2. **Body size limits**: a ~2-hour transcript is ~100 KB+ of Markdown. What
   does create/update answer? (Humla surfaces the status and size rather than
   truncating — same stance as the webhook's 413.)
3. **`name` vs leading `# ` heading**: confirm the object doesn't end up with
   a duplicated title (we send no H1; the name carries it).
4. **429 behaviour** under the paced queue — expected absent, but confirm the
   error body shape matches `{ code, message }`.
5. **Anytype closed** mid-bulk: confirm the connect-refused mapping to
   "Anytype isn't running" is the only transport error a user will realistically
   hit.
6. **Key scoping**: what a space-scoped / read-only key answers on create
   (401 vs 403) — the message tells the user to make a new key either way.
