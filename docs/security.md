# Security model

| Topic | Design |
|---|---|
| Network exposure | The API binds to 127.0.0.1 only. Remote management is planned for v2 with TLS and AD / Entra ID. |
| Authentication | 256-bit random token in `agent.token`. Compared in constant time. |
| Token access | File ACL: SYSTEM, Administrators, the **Keeprun Users** group and the installing user. Inheritance is removed. |
| Browser access | CORS allows only the Tauri app origins (plus the Vite dev server in dev). A web page cannot call the API, and it cannot read the token anyway. |
| Secrets | Env values marked secret are encrypted with Windows DPAPI under the agent account before they reach the database. The API returns them to authenticated clients so they can be edited. |
| Process isolation | Each app runs in its own Job Object. Stop or an agent crash ends the whole tree. |
| Service account | LocalSystem. Apps inherit it. Running apps as other accounts is planned (see below). |

## Not in v1

- **Per-app "Run as" account.** It needs stored passwords or gMSA support and
  `CreateProcessAsUser`, which deserve their own design and review. Until then, run apps that
  need fewer rights through a wrapper they already support, or host them on a dedicated VM.
- **Audit log of who did what.** Events record what happened to an app, not which user caused
  it, because all local clients share one token. Planned with per-user auth in v2.
