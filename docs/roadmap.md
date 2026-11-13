# Roadmap

These are the features planned after v1. The order is not fixed: votes and real use cases
decide what ships next.

**Vote:** each item has a GitHub issue with the
[roadmap label](https://github.com/yobotn/keeprun/issues?q=is%3Aissue+label%3Aroadmap+sort%3Areactions-%2B1-desc).
Add a thumbs up to the ones you want, and comment with how you would use it. Missing something?
[Open an issue](https://github.com/yobotn/keeprun/issues/new).

Shipped from this list: the [MCP server for coding agents](mcp.md).

## Run each app as its own account

[Vote on issue #2](https://github.com/yobotn/keeprun/issues/2)

Run an app as a chosen Windows account, such as a gMSA or a local service account, instead of
LocalSystem, so each app gets only the rights it needs. See the [security model](security.md).

## Reverse proxy with host names and HTTPS

[Vote on issue #3](https://github.com/yobotn/keeprun/issues/3)

Reach apps by internal host names, such as `hr.contoso.local`, over HTTPS with your company
certificate, without setting up IIS or nginx.

## Deploy from Git, a zip or a network share

[Vote on issue #4](https://github.com/yobotn/keeprun/issues/4)

Update an app from a Git branch, a zip file or a network share. Previous versions are kept so you
can roll back with one click or one command.

The first step has shipped: **Pull and restart** on the Overview tab and `keeprun pull <app>`
fast-forward the app's Git branch, run its install task when dependencies changed, and restart
it. Still planned: zip files and network shares, scheduled or webhook-triggered updates, and
rollback to the previous version.

## Several servers, directory sign-in and an audit log

[Vote on issue #5](https://github.com/yobotn/keeprun/issues/5)

Manage apps on several Windows servers from one desktop app. Sign in with Active Directory or
Entra ID, assign roles, and see who did what in an audit log.
