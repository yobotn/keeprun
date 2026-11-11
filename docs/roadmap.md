# Roadmap

These are the features planned after v1. The order is not fixed: votes and real use cases
decide what ships next.

**Vote:** each item has a GitHub issue with the
[roadmap label](https://github.com/yobotn/keeprun/issues?q=is%3Aissue+label%3Aroadmap+sort%3Areactions-%2B1-desc).
Add a thumbs up to the ones you want, and comment with how you would use it. Missing something?
[Open an issue](https://github.com/yobotn/keeprun/issues/new).

## MCP server for coding agents

A `keeprun mcp` command that runs a local MCP server, so coding agents and MCP clients such as
Claude Code, Claude Desktop and IDE agents can check status, read logs and restart apps without
shell access.

- Read-mostly by default: list, status, logs, events, start, stop and restart. Adding, removing,
  changing environment variables and running commands only when you turn them on.
- Secret values are never returned.
- Uses the existing local API, so there is no new port or service.

## Run each app as its own account

Run an app as a chosen Windows account, such as a gMSA or a local service account, instead of
LocalSystem, so each app gets only the rights it needs. See the [security model](security.md).

## Reverse proxy with host names and HTTPS

Reach apps by internal host names, such as `hr.contoso.local`, over HTTPS with your company
certificate, without setting up IIS or nginx.

## Deploy from Git, a zip or a network share

Update an app from a Git branch, a zip file or a network share. Previous versions are kept so you
can roll back with one click or one command.

## Several servers, directory sign-in and an audit log

Manage apps on several Windows servers from one desktop app. Sign in with Active Directory or
Entra ID, assign roles, and see who did what in an audit log.
