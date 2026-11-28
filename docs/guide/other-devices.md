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

1. On the Mac, open a project and choose **Oxplow ▸ Devices…** (or
   Settings ▸ Devices): the Devices page.
2. Tick **Let other devices use oxplow on this computer** and pick the
   Mac's address (the one on your home network, say `192.168.1.20`). The
   port is 7400 unless you set another. **Save**: it is on at once, and the
   page says the address a device opens. On many home networks the Mac's
   `.local` name works too, and keeps working when the address changes.
   Other names your network gives the Mac and a certificate of your own are
   under **Advanced**. (`oxplow --device-port <port>` or
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
4. On the tablet, open `https://192.168.1.20:7400/`, name it, and choose
   **Show a Code**. Type that code on the Mac, under **Connect a device**
   on the Devices page. The tablet then goes on by itself. A code lasts ten
   minutes.

The tablet then lists the projects; **Open** one to use it. Closing a
project on the tablet lets go of it there; it keeps running while the Mac
or another device has it open.

**Quitting Oxplow on the Mac stops it for every device.** When another
device is using it, Quit asks first and names it. The tablet says Oxplow
was quit, and connects again when Oxplow starts.

The Devices page also lists the paired devices and which are connected:
**Rename…** one, or **Remove** it (it must pair again). It works from the
tablet too.

## Several devices at once

Every device sees the same streams, threads, agents, terminals and tasks.
Each device keeps its own place: the stream and thread it shows, its tabs
of each thread, its panel layout, its preferences and its recent pages. So
the tablet can watch an agent's thread while the Mac edits in another.

- **Terminals are shared.** The device that types into a terminal sets
  its size. Another device shows it at that size, in a smaller font if it
  has less room, with a **Sized for another device** chip; **Fit to this
  device** takes the size over.
- **One window per device.** A device has one window for each project.
  Opening the project in a second browser tab moves it there: the first
  tab says Oxplow is open in another tab, and **Use it here** takes it
  back. Two browsers, or two addresses of the same Mac, count as two
  devices.
- **Continue from another device.** On the tablet layout, the ☰ menu's
  **Continue from** lists the other devices' tabs of the thread you show,
  and opens one here.
- **Agents open pages where you are.** An agent shows a page on the device
  you used last, or on the one you name ("show it on the iPad"). A device
  that is away gets the tab when it comes back.

On a tablet or phone, **View ▸ Tablet Layout** draws Oxplow for touch:
the stream, thread and tabs on one line at the top, the page at full
width, the panels in a tray at the bottom. **View ▸ Desktop Layout** goes
back. On a touch screen a long press is a right-click, a terminal has a
bar with the keys an on-screen keyboard lacks (Esc, Tab, Shift+Tab, Ctrl,
the arrows), and the code editor has selection keys under it.

## Run Oxplow on a server

On the server (an EC2 box, a homelab machine), with a project that is
already an Oxplow project (it has a `.oxplow/` folder):

```bash
oxplow host project add ~/src/myproject
oxplow host settings set allow=on address=<private-address>
oxplow host serve --bind 127.0.0.1:7400
```

Other devices come in on port 7400 (`port=<port>` chooses another). The
settings are kept and apply at once on a running host: `oxplow host
settings` shows them, and the Devices page changes them too. Flags given to
`serve` (`--public-bind <address>`, `--device-port`, `--allow-host <name>`,
`--tls-cert`/`--tls-key`) replace them for that run only.

It makes its own certificate as the Mac does; a device trusts it once from
`https://<private-address>:7400/host/ca.pem`. For a name that is not on a
private network, give a certificate of your own (`tls-cert=`, `tls-key=`,
signed by any CA) and the name (`names=<name>`).

This host runs until you stop it (`oxplow host stop`). Run it under
systemd or tmux to keep it past your SSH session. `oxplow host pair
<code>` pairs a device by the code it shows (`oxplow host pair` alone lists
who is waiting); `oxplow host devices` lists the paired devices.
To build it yourself, see
[`DEV.md`](https://github.com/nvoxland/oxplow/blob/main/DEV.md) "Run
headless".

From a browser, open `https://<private-address>:7400/`, choose **Show a
Code**, and run `oxplow host pair <that code>` on the server. From the Mac
app, the launcher's **Other computers** takes the server's address and
shows a code to run there the same way, then lists its projects to open. Over an SSH tunnel
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
