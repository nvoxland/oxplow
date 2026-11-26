# Other devices and computers

Oxplow on your Mac can be used from a tablet, a phone or another
computer's browser at the same time, and the Mac app can use Oxplow
running on a server. The agents and terminals are shared; each device
keeps its own place (its stream, its tabs, its layout).

There are no accounts. Each device pairs once with a short code, and the
link runs over a private network you already trust: Tailscale,
WireGuard, or an SSH tunnel.

## Use your Mac from a tablet

1. On the Mac, choose **Oxplow ▸ Devices…**.
2. Under **Allow other devices**, turn it on and set the address other
   devices reach the Mac by. With Tailscale, **Use Tailscale** fills in
   the Mac's tailnet address and name.
3. For the microphone and the clipboard the tablet's browser needs
   HTTPS. With Tailscale, `tailscale cert <your-mac>.<tailnet>.ts.net`
   writes a certificate and key; put their paths in the two fields.
4. **Save**, then quit Oxplow and open it again: the settings apply at
   the next start.
5. Choose **Pair a Device**. On the tablet, open the address the window
   shows (`https://<your-mac>…:7400/`) and type the code. It is good once,
   for ten minutes.

The tablet then lists the projects; **Open** one to use it. Closing a
project on the tablet lets go of it there; it keeps running while the Mac
or another device has it open.

**Quitting Oxplow on the Mac stops it for every device.** When another
device is using it, Quit asks first and names it. The tablet says Oxplow
was quit, and connects again when Oxplow starts.

Devices… also lists the paired devices: rename one in place, or
**Remove** it (it must pair again).

## Run Oxplow on a server

On the server (an EC2 box, a homelab machine), with a project that is
already an Oxplow project (it has a `.oxplow/` folder):

```bash
oxplow host project add ~/src/myproject
oxplow host serve --bind 127.0.0.1:7400 \
  --public-bind <tailnet-address>:7400 --allow-host <server>.<tailnet>.ts.net \
  --tls-cert server.crt --tls-key server.key
```

This host runs until you stop it (`oxplow host stop`). Run it under
systemd or tmux to keep it past your SSH session. `oxplow host pair`
prints a pairing code; `oxplow host devices` lists the paired devices.
To build it yourself, see
[`DEV.md`](https://github.com/nvoxland/oxplow/blob/main/DEV.md) "Run
headless".

From a browser, open `https://<server>…:7400/` and type the code. From
the Mac app, the launcher's **Other computers** takes the server's
address and a code, then lists its projects to open. Over an SSH tunnel
instead (`ssh -L 7400:127.0.0.1:7400 server`, the host on
`--bind 127.0.0.1:7400`), the address is `http://127.0.0.1:7400`. A project there
runs there: quitting the Mac app leaves it running, and its window comes
back at the next start.

## Current limitations

- **External URL tabs** open in the browser of the device you are on.
- **Native menus and the clipboard** are the device's own.
- Latency is your network's. Terminal echo over a slow link feels like
  SSH because it effectively is.
- A window on another computer's project shows no custom component
  frames yet.
- The Mac app and a host on the same Mac can't both run one home's
  projects: use another `OXPLOW_HOME`, or connect to it as another
  computer.
