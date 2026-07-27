# Halogen Privacy Policy

_Last updated: 2026-07-27_

Halogen is a self-hosted podcast client. This policy covers the Halogen iOS
app and is intentionally short, because the honest summary is: **we collect
nothing.**

## Data we collect

None. The app has no analytics, no crash reporting, no advertising, no
third-party SDKs, and no backend operated by us. Nothing you do in the app is
transmitted to the developer or to any partner.

## Where your data lives

- **On your device.** Subscriptions, playback positions, downloads, settings,
  and (if you use the built-in library) the embedded server's database are
  stored on your device only. Sign-in credentials and session tokens are kept
  in the iOS Keychain.
- **On your own server.** If you connect the app to a server you host, your
  podcast library and listening state sync to that server. That server is
  yours: its data handling is under your control, not ours.

## Network connections the app makes

- To the server **you** configure (yours, or the on-device embedded server).
- Through that server, to the podcast feeds and directories you ask it to
  fetch (RSS feeds, episode audio, artwork, and the iTunes/gpodder.net
  search directories when you use Discover). The app itself talks only to
  your server.

## Data deletion

Everything can be deleted in-app: individual downloads, cached data, accounts
on the embedded server, or the entire embedded library (Settings → Local
data). Deleting the app removes all app data from the device. Data on a
server you host is deleted by you, on that server.

## Tracking

The app does not track you, fingerprint you, or share anything with data
brokers. There is nothing to opt out of.

## Changes

If a future version ever changes any of the above, this policy will be
updated first and the change called out in the release notes.

## Contact

Questions: open an issue on the project repository.
