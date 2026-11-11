# Privacy

Keeprun is built to keep your data on your own machines. This page explains what the software
and this website do with data.

## The Keeprun software

- **No telemetry.** Keeprun does not collect or send usage data, crash reports, app names, logs
  or any other information about your machines or apps.
- **No account.** There is nothing to sign up for and no cloud service.
- **Local only.** The agent's API listens on 127.0.0.1 and is not reachable from the network.
  App settings, logs and events are stored in the Keeprun data folder on the same machine,
  `C:\ProgramData\Keeprun` for the installed service.
- **Secrets stay encrypted.** Environment values marked secret are encrypted with Windows DPAPI
  before they are stored, and are kept out of Keeprun's own logs.
- **Update checks.** The only outbound request is the optional update check. The desktop app
  downloads `https://keeprun.app/latest.json`, which reveals your IP address and a standard
  request header to the website host, like any web request. It sends no identifier. You can
  turn update checks off, or point them at an internal mirror, with an MSI property or Group
  Policy. See the [deployment guide](/docs/deployment.html).

## This website

keeprun.app is a static site hosted on GitHub Pages. It sets no cookies, has no analytics, ads
or tracking scripts, and has no forms. GitHub, as the host, receives standard request data such
as IP addresses to serve and protect the site. See the
[GitHub privacy statement](https://docs.github.com/site-policy/privacy-policies/github-general-privacy-statement).

Downloads are served from GitHub Releases, and issues and discussions happen on GitHub, under
GitHub's terms and privacy statement.

## Changes and questions

Changes to this page are visible in the
[repository history](https://github.com/{{REPO}}/commits/main/site/pages/privacy.md). For
questions, see [contact](/contact).
