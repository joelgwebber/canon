# Firewalls

Speakers connect *in* to canon. A Chromecast or DLNA renderer is handed a URL on this machine and
fetches the audio from it over TCP, and DLNA devices answer canon's searches with unicast UDP. Both
go to one port, **7346** (`--lan-port` / `CANON_LAN_PORT` to change it), so one rule admits both.

A firewall that drops them doesn't announce it. Cast speakers accept a track and never play it
("It never fetched a stream from http://…:7346/…"), and DLNA devices never appear in `sinks` or
`canon devices`. Chromecasts are still listed, because they're found over multicast mDNS, which
default firewall rules usually let in.

## ufw (Linux)

ufw is on by default on some distributions (CachyOS among them) and drops anything inbound that no
rule allows. `canon serve` reads ufw's state when it starts and, if nothing admits its port, logs
the exact command to run, with your network filled in. It's this, once:

```sh
sudo ufw allow from 192.168.0.0/24 to any port 7346
```

Or install canon's application profile and allow that by name, which keeps `ufw status` readable:

```sh
sudo cp packaging/ufw/canon /etc/ufw/applications.d/canon
sudo ufw allow from 192.168.0.0/24 to any app canon
```

The profile names port 7346. If you change `--lan-port`, allow that port instead.

`--lan-port 0` picks a new port every time, which no rule can name; use it only without a firewall.

## Two canons on one machine

Only one process can own a port. A second daemon (a test one beside your own), or
`canon devices` while `canon serve` is running, finds 7346 taken. It then uses a port the OS
picks, logging a warning, and a firewall drops that port's traffic too. Give the second daemon a
port of its own (`--lan-port 7347`) and allow it the same way.

## firewalld (Fedora, openSUSE)

Fedora Workstation's default zone already admits ports above 1024, so canon works as it is. In a
stricter zone:

```sh
sudo firewall-cmd --permanent --add-port=7346/tcp --add-port=7346/udp && sudo firewall-cmd --reload
```

## macOS

The Application Firewall works per application, not per port: see
[Signing dev builds](../AGENTS.md#signing-dev-builds).
