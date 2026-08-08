#!/bin/bash
# tailscaled + remote-access services for the devboxvm flow. INSTALL ONLY, no
# login: a baked-in key would put every throwaway release clone on the tailnet;
# tart-create-devboxvm.sh does the one-time `tailscale up` in the persistent clone.
set -euo pipefail
eval "$(/opt/homebrew/bin/brew shellenv)"
export HOMEBREW_NO_AUTO_UPDATE=1

echo "==> tailscale (brew) + system daemon (boots logged-out until first 'up')"
brew install tailscale
sudo /opt/homebrew/bin/tailscaled install-system-daemon

echo "==> remote access: Remote Login (ssh :22) + Screen Sharing (vnc :5900)"
# The cirruslabs base ships both enabled — keep this idempotent/best-effort so
# a future base image flipping a default doesn't silently lose remote access.
sudo launchctl load -w /System/Library/LaunchDaemons/ssh.plist 2>/dev/null || true
sudo launchctl load -w /System/Library/LaunchDaemons/com.apple.screensharing.plist 2>/dev/null || true
# Legacy VNC password (= VM password) so Linux viewers without Apple-DH auth
# (e.g. TigerVNC) can connect; libvncclient-based ones can use admin/admin.
sudo /System/Library/CoreServices/RemoteManagement/ARDAgent.app/Contents/Resources/kickstart \
  -configure -clientopts -setvnclegacy -vnclegacy yes -setvncpw -vncpw admin 2>/dev/null || true
