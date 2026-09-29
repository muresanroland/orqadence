# On call

On call lets a run go on while you are away from the desk. Your phone rings when
a Question waits, and you answer it in the Shell over SSH.

## What On call does

When a Question has waited 5 minutes unanswered, the Shell goes On call and
rings your phone through [Moshi](https://getmoshi.app). The minutes are set on
the On call page of `/config`. When On call starts, every Question already
waiting is pushed once. While it is on, every Question after it and the end of
the run are pushed to the phone at once. The status row shows ON CALL.

You answer in the Shell, from the phone over SSH. Answering any Question ends
On call, whichever Question it is and wherever you typed it. The next Question
rings only after it has itself waited the minutes.

Nothing parks for On call: a Stage's question waits and rings. `/away` wins:
turning Away on ends On call, and under Away nothing rings and the Shell never
goes On call. On call is off each
time the Shell opens, and stays off while no token is kept.

## Moshi

1. Install the Moshi app on your phone.
2. In Moshi, open Settings > Notifications and copy the webhook token.
3. Give the token to Orqadence, one of three ways:
   - `orqa init` asks "Ring your phone through Moshi when a Question waits?".
     Say yes and paste the token.
   - In the Shell, open `/config`, go to the On call page and type the token.
   - Set `MOSHI_WEBHOOK_TOKEN` in the environment the Shell runs in.
4. On the On call page of `/config`, pick "Send a test push". The phone should
   ring almost at once.

## Reaching the Shell

1. On the Mac, turn on Remote Login: System Settings > General > Sharing.
2. In Moshi, add the Mac as an SSH host: the user is your Mac login name, the
   password is its login password, the port is 22.
3. Connect and type `herdr`. It attaches to the running herdr session, where
   the Shell is. Answer the Question there.

Plain SSH is free in Moshi. mosh and Moshi's herdr integration need Moshi Pro;
you do not need either.

## Away from home

At home, Moshi reaches the Mac by its `.local` name or its LAN IP. Neither works
from anywhere else. Use [Tailscale](https://tailscale.com):

1. Install Tailscale on the Mac and on the phone.
2. Put the phone on the same tailnet as the Mac: the same account, signed in
   with the same login provider. `tailscale status` on the Mac lists the phone
   as a peer when it is.
3. On the phone, turn on the VPN toggle in the Tailscale app.
4. In Moshi, set the host to the Mac's Tailscale IP or its MagicDNS name. The
   user, password and port stay the same.

## The token and the push

The token is kept in `.orqadence-local/config.json`, readable only by you.
`MOSHI_WEBHOOK_TOKEN` in the environment wins over the file.

A push says which Ticket waits and on what: its id, the kind of Question and the
Ticket's title, for example
`harness-bsg.4 · Plan to approve · Brainstorm loop: ...`. The Question's text
and its options never leave the Mac. A push that fails shows as a notice line
in the Shell; the run goes on.

## Pitfalls

- **Remote Login is off.** Moshi cannot connect. Turn it on in System
  Settings > General > Sharing.
- **The clipboard holds a command, not the token.** Copying a command such as
  `orqa init` after the token replaces the token on the clipboard, and you
  paste the command as the token. Copy the token last, just before you paste
  it, and send a test push to check.
- **The Mac is asleep, or the Shell is not running.** Nothing rings: the Shell
  is what pushes. Keep the Mac awake and the Shell open while you are away.
- **The phone is off the tailnet.** Away from home, Moshi cannot reach the Mac
  until Tailscale's VPN toggle is on and the phone is on the Mac's tailnet.
