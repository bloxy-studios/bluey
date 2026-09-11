# Fingerprint fixtures

Scrubbed captures of the official clients' requests (ADR 0009) — the data behind
`bun run fingerprints:diff <provider>`. Runbook: `docs/PROVIDER_ACCOUNTS.md › Re-capture runbook`.

```
<provider>/                     chatgpt · claude · antigravity
  documented/<endpoint>.json    the tables of docs/PROVIDER_ACCOUNTS.md as a capture — generated
                                from bluey_protocols::fingerprints by
                                `UPDATE_FIXTURES=1 cargo test -p bluey-protocols fingerprints`
  golden/<endpoint>.json        a reviewed real capture, promoted with `bun run fingerprints:bless`
  captures/<ts>-<endpoint>.json fresh captures from `fingerprints:capture` / `import-har`
                                (git-ignored — local by design)
```

Every file is one `Capture` (`bluey_protocols::fingerprints::capture`, schema 1):

```json
{
  "schema": 1,
  "provider": "claude",
  "source": "proxy | har | documented | probe",
  "capturedAt": "2026-09-11T10:00:00Z",
  "fingerprint": { "version": "claude_code/2.1.258", "capturedOn": "2026-09-11" },
  "client": "claude-cli/2.1.258",
  "request": { "method": "POST", "url": "https://api.anthropic.com/v1/messages?beta=true",
               "headers": [{ "name": "authorization", "value": "Bearer <ACCESS_TOKEN>" }],
               "body": { "kind": "json", "value": { … } } },
  "response": { "status": 200, "headers": [ … ], "body": { "kind": "sse", "events": [ … ] },
                "durationMs": 812, "truncated": false },
  "scrubbed": ["authorization", "user_text", "uuid"],
  "notes": [ … ]
}
```

Placeholders are stable so a real capture and the documented one compare directly:
`<ACCESS_TOKEN>` `<REFRESH_TOKEN>` `<API_KEY>` `<JWT>` `<EMAIL>` `<UUID>` `<HEX64>` `<HEX32>`
`<ID>` `<ACCOUNT_UUID>` `<PROJECT_ID>` `<SESSION_ID>` `<TEXT n>` `<BASE64 n>` `<OPAQUE n>`
`/Users/<USER>`. Fingerprint blocks (identity text, billing header, wrapper fields) are kept verbatim.
A Vitest test (`tests/unit/accounts/fingerprint-fixtures.test.ts`) refuses secret-shaped strings in
any committed fixture.
