# Other devices and computers

Oxplow on your Mac can be used from a tablet, a phone or another
computer's browser at the same time, and the Mac app can use Oxplow
running on a server. The agents and terminals are shared; each device
keeps its own place (its stream, its tabs, its layout).

There are no accounts and nothing to install. Each device pairs once
with a short code, over HTTPS with a certificate Oxplow makes itself. The
devices only need to reach each other: the same home network is enough, and
so is a private network such as Tailscale or WireGuard.

## Use your Mac from a tablet

1. On the Mac, choose **Oxplow ▸ Devices…**.
2. Under **Allow other devices**, turn it on and pick the Mac's address
   (the one on your home network, say `192.168.1.20`) from the list. To
   open it by name instead, choose **Use <your Mac>.local** under *Also
   reach this Mac by name*; a device that cannot find the name uses the
   address. The port is 7400
   unless you set another. **Save**, then quit Oxplow and open it again:
   the settings apply at the next start. (`oxplow --device-port <port>` or
   `OXPLOW_DEVICE_PORT` set the port for one run, over the setting.)
3. The first time on a tablet, make it trust the Mac. Open
   `https://192.168.1.20:7400/host/ca.pem` there (past the browser's
   warning) and install it:
   - Android: Settings ▸ Security ▸ More ▸ Install a certificate ▸ CA
     certificate.
   - iPhone, iPad: install the downloaded profile, then turn it on under
     Settings ▸ General ▸ About ▸ Certificate Trust Settings.

   The certificate can vouch only for addresses and names on private
   networks, never for a real website.
4. Choose **Pair a Device** on the Mac. On the tablet, open
   `https://192.168.1.20:7400/` and type the code. It is good once, for
   ten minutes.

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
oxplow host serve --bind 127.0.0.1:7400 --public-bind <private-address>
```

Other devices come in on port 7400; `--device-port <port>` or
`OXPLOW_DEVICE_PORT` choose another.

It makes its own certificate as the Mac does; a device trusts it once from
`https://<private-address>:7400/host/ca.pem`. For a name that is not on a
private network, give a certificate of your own (`--tls-cert`,
`--tls-key`, signed by any CA) and `--allow-host <name>`.

This host runs until you stop it (`oxplow host stop`). Run it under
systemd or tmux to keep it past your SSH session. `oxplow host pair`
prints a pairing code; `oxplow host devices` lists the paired devices.
To build it yourself, see
[`DEV.md`](https://github.com/nvoxland/oxplow/blob/main/DEV.md) "Run
headless".

From a browser, open `https://<private-address>:7400/` and type the code. From
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
