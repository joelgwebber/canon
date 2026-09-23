//! Resilient, supervised device discovery (yak canon-ea5d).
//!
//! This is the root-cause fix for tideway's discovery flakiness (tide-6fd0/8f5b), where
//! renderers would appear, vanish, and flap on the picker. Two distinct bugs hid under that
//! one symptom, and both are addressed structurally here rather than papered over:
//!
//! * **A stale `utun` route silently black-holed multicast.** tideway enumerated *every*
//!   interface and let the OS pick the multicast egress. After a VPN came and went, a leftover
//!   `224.0.0.0/4` route on a dead `utun` interface won the route lookup, so every mDNS query
//!   left through an interface with no listeners and no answers ever came back — discovery just
//!   went quiet with no error. The fix is [`usable_interfaces`]: enumerate, *exclude tunnels /
//!   VPNs / loopback / link-local*, prefer real LAN NICs, and then **pin** multicast egress to
//!   the chosen interface (`IP_MULTICAST_IF`) instead of trusting the route table. That filter
//!   is the crown jewel of this module and is pure-unit-tested against a synthetic table.
//! * **A wholesale per-scan replace made devices flap.** tideway rebuilt its device list from
//!   scratch on every scan, so a device that missed a single response round blinked out and
//!   back. Here a long-lived [`DiscoveryService`] owns its **own cache** with add / remove /
//!   TTL-expiry events, and publishes a *debounced* snapshot: a device only disappears when it
//!   is explicitly removed *or* its liveness TTL lapses — never because one scan happened not
//!   to hear it. The snapshot is coalesced so a burst of churn yields a single update.
//!
//! The mDNS machinery sits behind a thin boundary ([`CastSource`]) so the cache, debounce, and
//! interface filter — the parts that actually encode the tideway lessons — are testable without
//! ever binding a socket or spinning the real daemon. The supervisor / cache shape is
//! deliberately protocol-agnostic: DLNA/SSDP (yak canon-685a) plugs in later as a *second*
//! source feeding the same [`DeviceCache`], with no change to the debounce or publish path.

use std::collections::HashMap;
use std::net::{IpAddr, Ipv4Addr, SocketAddr, SocketAddrV4};
use std::time::{Duration, Instant};

use canon_core::{Error, Result, SinkId, SinkKind};
use mdns_sd::{IfKind, ResolvedService, ServiceDaemon, ServiceEvent};
use socket2::{Domain, Protocol, SockAddr, Socket, Type};
use tokio::sync::{mpsc, watch};
use tokio::task::JoinHandle;

/// The mDNS service type Chromecast / Google Cast devices advertise.
const CAST_SERVICE: &str = "_googlecast._tcp.local.";
/// The IPv4 mDNS multicast group (RFC 6762). We (re)join this per chosen NIC so that, after a
/// wake, group forwarding is re-established on the *right* interface.
const MDNS_GROUP_V4: Ipv4Addr = Ipv4Addr::new(224, 0, 0, 251);
/// How long an entry survives without being refreshed before its liveness lapses.
///
/// `None` disables expiry, which is correct for the mDNS source: `mdns-sd` keeps its own record
/// cache, re-queries on its own schedule, and emits `ServiceRemoved` when a record genuinely
/// expires — but it only re-emits `ServiceResolved` on a *change*, so a still-present device that
/// it silently refreshed produces no event here. A blind TTL on top of that therefore reaps live
/// devices (observed: the list emptied after two minutes and never recovered). The source that
/// knows about liveness drives removal; we don't second-guess it.
///
/// Kept as an option rather than deleted because a polled protocol — SSDP, whose `M-SEARCH`
/// responses carry their own `CACHE-CONTROL` lifetime — genuinely does need expiry here.
const DEFAULT_TTL: Option<Duration> = None;
/// The window over which a burst of cache changes is coalesced into one published snapshot.
/// Long enough to swallow the multi-packet resolve of a single device; short enough to feel
/// live in the picker.
const DEFAULT_DEBOUNCE: Duration = Duration::from_millis(300);

// ── Public data model ────────────────────────────────────────────────────────────────────

/// A device the discovery service currently believes is reachable.
///
/// This is the picker's view: stable [`id`](Self::id) keyed to the device (so it doesn't flap),
/// a friendly [`name`](Self::name), the protocol [`kind`](Self::kind), and the [`addr`](Self::addr)
/// to reach it. Additional protocol-specific detail (TXT records, model) can hang off later
/// without disturbing this shape.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DiscoveredDevice {
    /// Stable identity for this device, consistent across resolve / remove / expiry so the
    /// cache dedupes and the picker never sees a device blink.
    pub id: SinkId,
    /// Human-facing name (Cast TXT `fn`, falling back to the mDNS instance label).
    pub name: String,
    /// Which protocol found it, for grouping and for choosing the right `Sink` impl.
    pub kind: SinkKind,
    /// Where to reach it (first IPv4 address if any, else IPv6, with the advertised port).
    pub addr: SocketAddr,
}

/// A long-lived, supervised discovery service.
///
/// Owns the cache and the debounce timer; publishes a deduped, stably-sorted snapshot through a
/// [`watch::Receiver`] that a picker reads. Construct with [`spawn`](Self::spawn) (needs a Tokio
/// runtime); the underlying daemon and its bridge task are torn down when the service is dropped.
#[derive(Debug)]
pub struct DiscoveryService {
    devices: watch::Receiver<Vec<DiscoveredDevice>>,
    resync_tx: mpsc::Sender<()>,
    task: JoinHandle<()>,
}

impl DiscoveryService {
    /// Spawn the service with default liveness / debounce timings.
    ///
    /// # Errors
    /// Returns [`Error::Sink`] if the mDNS daemon cannot be started or the Cast browse cannot be
    /// registered.
    pub fn spawn() -> Result<Self> {
        Self::spawn_with(DEFAULT_TTL, DEFAULT_DEBOUNCE)
    }

    /// Spawn with explicit TTL and debounce window (used to keep the timings honest and to make
    /// the knobs visible rather than buried as magic numbers).
    ///
    /// # Errors
    /// See [`spawn`](Self::spawn).
    pub fn spawn_with(ttl: Option<Duration>, debounce: Duration) -> Result<Self> {
        let (events_tx, events_rx) = mpsc::channel(256);
        let cast = CastSource::start(events_tx)?;

        let (dev_tx, dev_rx) = watch::channel(Vec::new());
        let (resync_tx, resync_rx) = mpsc::channel(8);
        let task = tokio::spawn(supervise(cast, events_rx, resync_rx, dev_tx, ttl, debounce));

        Ok(Self {
            devices: dev_rx,
            resync_tx,
            task,
        })
    }

    /// A live, debounced view of the currently-reachable devices. Cheap to clone; every clone
    /// observes the same snapshot stream.
    #[must_use]
    pub fn devices(&self) -> watch::Receiver<Vec<DiscoveredDevice>> {
        self.devices.clone()
    }

    /// Wake-safe resync hook: rebuild the multicast sockets (re-`IP_ADD_MEMBERSHIP` on the
    /// chosen NICs) and re-burst discovery.
    ///
    /// This is the seam a sleep/wake watchdog calls. The watchdog itself — a timer that watches
    /// for a suspiciously long silence on the receive path and calls `resync` when reception
    /// looks *wedged* (the classic post-wake failure: the socket is up, membership silently
    /// lapsed, no packets arrive) — is deliberately left for the on-metal pass, where "wedged"
    /// can be measured against real traffic. The mechanism it drives is fully built here.
    pub async fn resync(&self) {
        // A full channel means a resync is already queued; one is as good as two.
        let _ = self.resync_tx.try_send(());
    }
}

impl Drop for DiscoveryService {
    fn drop(&mut self) {
        // Abort the supervisor; dropping its future runs `CastSource`'s `Drop`, which shuts the
        // mDNS daemon and aborts the bridge task. No detached daemon thread is left behind.
        self.task.abort();
    }
}

// ── Interface filtering: the tideway root-cause fix ──────────────────────────────────────

/// The minimal facts [`usable_interfaces`] needs about a host interface, mapped from
/// `if_addrs::Interface` at the boundary. Kept deliberately tiny and if-addrs-free so the filter
/// is a pure function tested against a hand-written table rather than whatever NICs the test box
/// happens to have.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Iface {
    /// OS interface name, e.g. `en0`, `utun3`, `wg0`.
    pub name: String,
    /// One bound address on the interface.
    pub ip: IpAddr,
    /// Whether this is the loopback interface.
    pub is_loopback: bool,
}

/// Interface-name prefixes that denote a tunnel / VPN. These are exactly what black-holed
/// tideway's multicast: a VPN leaves a stale `224.0.0.0/4` route pointing at a dead `utun`, and
/// the OS then routes every mDNS query into the void. We refuse to consider them at all.
const TUNNEL_PREFIXES: &[&str] = &["utun", "tun", "tap", "ppp", "ipsec", "wg"];
/// Interface-name prefixes for real wired / wireless LAN NICs, which we prefer for multicast.
const LAN_PREFIXES: &[&str] = &["en", "eth", "wlan"];

/// Select the interfaces worth doing LAN discovery on, in preference order.
///
/// Excludes tunnels/VPNs (by name), loopback, and link-local addresses (169.254.0.0/16 and
/// fe80::/10 — addresses that never reach a LAN renderer), then orders real LAN NICs
/// (`en*`/`eth*`/`wlan*`) ahead of any surviving oddballs. The head of the returned list is what
/// multicast egress is pinned to; keeping the oddballs as a tail rather than dropping them means
/// a validly-addressed but unusually-named NIC still works as a fallback, while the stale-`utun`
/// black hole — the actual tideway bug — can never be chosen.
#[must_use]
pub fn usable_interfaces(all: &[Iface]) -> Vec<Iface> {
    let mut usable: Vec<Iface> = all
        .iter()
        .filter(|i| !i.is_loopback)
        .filter(|i| !is_tunnel(&i.name))
        .filter(|i| !is_link_local(i.ip))
        .cloned()
        .collect();

    // Stable partition: preferred LAN NICs first, everything else after, each group keeping the
    // caller's original order (a stable sort, so it is a genuine preference, not a reshuffle).
    usable.sort_by_key(|i| u8::from(!is_lan(&i.name)));
    usable
}

/// True if the interface name marks a tunnel / VPN we must never route discovery through.
fn is_tunnel(name: &str) -> bool {
    TUNNEL_PREFIXES.iter().any(|p| name.starts_with(p))
}

/// True if the interface name is a real wired/wireless LAN NIC we prefer.
fn is_lan(name: &str) -> bool {
    LAN_PREFIXES.iter().any(|p| name.starts_with(p))
}

/// True for link-local addresses (IPv4 169.254.0.0/16, IPv6 fe80::/10), which are auto-config
/// addresses that don't reach a real LAN renderer. `Ipv6Addr::is_unicast_link_local` is still
/// unstable, so the IPv6 case is computed directly per RFC 4291.
fn is_link_local(ip: IpAddr) -> bool {
    match ip {
        IpAddr::V4(v4) => v4.is_link_local(),
        IpAddr::V6(v6) => (v6.segments()[0] & 0xffc0) == 0xfe80,
    }
}

// ── The cache: add / remove / TTL-expiry, protocol-agnostic ──────────────────────────────

/// A discovery observation fed into the cache by *any* protocol source. DLNA/SSDP will emit the
/// same two events; the cache neither knows nor cares which protocol produced them.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum DiscoveryEvent {
    /// A device was resolved (first sighting or a liveness-refreshing re-announcement).
    Added(DiscoveredDevice),
    /// A source explicitly reported a device gone.
    Removed(SinkId),
}

/// One cached device plus the instant its liveness lapses (`None` when the source drives removal).
#[derive(Debug, Clone)]
struct CacheEntry {
    device: DiscoveredDevice,
    expires_at: Option<Instant>,
}

/// The supervisor's own device cache. Deduped by [`SinkId`]; entries carry a TTL and expire on
/// their own so a device that stops announcing is removed by *liveness*, not left to rot or
/// dropped on the next missed scan.
#[derive(Debug)]
pub(crate) struct DeviceCache {
    entries: HashMap<SinkId, CacheEntry>,
    /// `None` means entries never expire on a timer; the source's remove events are authoritative
    /// (see [`DEFAULT_TTL`]).
    ttl: Option<Duration>,
}

impl DeviceCache {
    fn new(ttl: Option<Duration>) -> Self {
        Self {
            entries: HashMap::new(),
            ttl,
        }
    }

    /// Insert or refresh a device. Returns whether the *content* of the cache changed (a new
    /// device, or changed fields) — a bare liveness refresh of an unchanged device returns
    /// `false`, so a device re-announcing on schedule doesn't churn the published snapshot.
    fn add(&mut self, device: DiscoveredDevice, now: Instant) -> bool {
        let expires_at = self.ttl.map(|ttl| now + ttl);
        match self.entries.get_mut(&device.id) {
            Some(existing) => {
                existing.expires_at = expires_at;
                if existing.device == device {
                    false
                } else {
                    existing.device = device;
                    true
                }
            }
            None => {
                self.entries
                    .insert(device.id.clone(), CacheEntry { device, expires_at });
                true
            }
        }
    }

    /// Drop a device by id. Returns whether anything was removed.
    fn remove(&mut self, id: &SinkId) -> bool {
        self.entries.remove(id).is_some()
    }

    /// Evict every entry whose TTL has lapsed at `now`. Entries with no expiry are never evicted
    /// here. Returns whether anything was evicted.
    fn expire(&mut self, now: Instant) -> bool {
        let before = self.entries.len();
        self.entries
            .retain(|_, e| e.expires_at.is_none_or(|expires| expires > now));
        self.entries.len() != before
    }

    /// The current snapshot: deduped (one entry per id) and stably sorted by name then id, so the
    /// picker sees a stable order rather than `HashMap` iteration noise.
    fn snapshot(&self) -> Vec<DiscoveredDevice> {
        let mut out: Vec<DiscoveredDevice> =
            self.entries.values().map(|e| e.device.clone()).collect();
        out.sort_by(|a, b| a.name.cmp(&b.name).then_with(|| a.id.0.cmp(&b.id.0)));
        out
    }
}

// ── Debounce: coalesce a burst of changes into one published snapshot ────────────────────

/// Cache + debounce as one pure, time-injectable component. The async supervisor drives it with
/// real `Instant`s and a Tokio timer; tests drive it with synthetic instants and no clock at all.
///
/// Debounce is *leading-edge armed*: the first change after a quiet period arms a deadline one
/// window out, and every further change until then is absorbed. That coalesces a burst (a single
/// device resolving across several packets, or a wake-storm of re-announcements) into exactly one
/// snapshot, and — unlike a deadline that resets on every change — guarantees the update actually
/// fires within one window instead of being starved by a steady trickle.
#[derive(Debug)]
pub(crate) struct SnapshotEngine {
    cache: DeviceCache,
    debounce: Duration,
    /// `Some(deadline)` while a coalesced update is pending; `None` when quiescent.
    deadline: Option<Instant>,
}

impl SnapshotEngine {
    pub(crate) fn new(ttl: Option<Duration>, debounce: Duration) -> Self {
        Self {
            cache: DeviceCache::new(ttl),
            debounce,
            deadline: None,
        }
    }

    /// Apply one discovery event, arming the debounce only if the cache content actually changed.
    pub(crate) fn apply(&mut self, event: DiscoveryEvent, now: Instant) {
        let changed = match event {
            DiscoveryEvent::Added(device) => self.cache.add(device, now),
            DiscoveryEvent::Removed(id) => self.cache.remove(&id),
        };
        if changed {
            self.arm(now);
        }
    }

    /// Run TTL expiry, arming the debounce if anything lapsed.
    pub(crate) fn tick_expiry(&mut self, now: Instant) {
        if self.cache.expire(now) {
            self.arm(now);
        }
    }

    /// The pending publish deadline, if any — the async loop sleeps until this.
    pub(crate) fn deadline(&self) -> Option<Instant> {
        self.deadline
    }

    /// If a coalesced update is now due, disarm and hand back the fresh snapshot; otherwise
    /// `None`. Fires at most once per burst.
    pub(crate) fn poll(&mut self, now: Instant) -> Option<Vec<DiscoveredDevice>> {
        if let Some(deadline) = self.deadline
            && now >= deadline
        {
            self.deadline = None;
            return Some(self.cache.snapshot());
        }
        None
    }

    /// Arm the leading-edge deadline if not already armed.
    fn arm(&mut self, now: Instant) {
        if self.deadline.is_none() {
            self.deadline = Some(now + self.debounce);
        }
    }
}

// ── The mDNS boundary: everything real-network lives behind this ─────────────────────────

/// The Cast (mDNS) discovery source: a thin wrapper over `mdns-sd` that enables only the chosen
/// interfaces, holds a pinned multicast membership anchor per NIC, and forwards resolved / removed
/// services into the cache as [`DiscoveryEvent`]s.
///
/// This is the seam that keeps the daemon out of the tested logic. Everything above — filter,
/// cache, debounce — is exercised without it; everything network-touching is confined here.
struct CastSource {
    daemon: ServiceDaemon,
    /// Interfaces we chose to discover on, in preference order.
    chosen: Vec<Iface>,
    /// One IP_MULTICAST_IF-pinned, group-joined socket per chosen IPv4 NIC. See
    /// [`bind_pinned_multicast_v4`] for why these exist even though `mdns-sd` owns its own
    /// sockets: they anchor group membership on the *specific* NIC so a wake re-join re-arms
    /// forwarding there, and they are the exact substrate the future SSDP source will send on.
    anchors: Vec<Socket>,
    /// The task translating `mdns-sd`'s flume channel into cache events.
    bridge: JoinHandle<()>,
}

impl CastSource {
    /// Start the daemon, pin it to the usable interfaces, and begin browsing for Cast devices.
    fn start(events: mpsc::Sender<DiscoveryEvent>) -> Result<Self> {
        let daemon = ServiceDaemon::new().map_err(|e| Error::Sink(format!("mdns daemon: {e}")))?;
        let chosen = usable_interfaces(&host_interfaces());

        // Discover on the chosen interfaces *only* — this is the mdns-sd equivalent of pinning
        // egress, and it is what keeps a stale utun from ever carrying our queries.
        pin_daemon_interfaces(&daemon, &chosen)?;
        let anchors = build_anchors(&chosen);

        let rx = daemon
            .browse(CAST_SERVICE)
            .map_err(|e| Error::Sink(format!("mdns browse: {e}")))?;

        let bridge = tokio::spawn(async move {
            // flume's async recv; the channel closes when the daemon shuts down.
            while let Ok(event) = rx.recv_async().await {
                if let Some(discovery) = to_event(event)
                    && events.send(discovery).await.is_err()
                {
                    break; // supervisor gone; nothing left to feed.
                }
            }
        });

        Ok(Self {
            daemon,
            chosen,
            anchors,
            bridge,
        })
    }

    /// Wake-safe rebuild: re-pin interfaces (forcing `mdns-sd` to re-announce/query — the
    /// re-burst), and tear down and recreate the pinned anchors so each NIC issues a fresh
    /// `IP_ADD_MEMBERSHIP`. The existing browse keeps delivering, so no device is dropped.
    fn rebuild(&mut self) {
        tracing::debug!(
            interfaces = self.chosen.len(),
            anchors = self.anchors.len(),
            "discovery resync: re-pinning interfaces and rebuilding multicast anchors"
        );
        if let Err(e) = pin_daemon_interfaces(&self.daemon, &self.chosen) {
            tracing::warn!(error = %e, "discovery resync: re-pinning interfaces failed");
        }
        // Drop the old anchors (leaving the groups) before rejoining, so the OS re-issues
        // membership rather than treating it as a no-op refresh.
        self.anchors.clear();
        self.anchors = build_anchors(&self.chosen);
    }
}

impl Drop for CastSource {
    fn drop(&mut self) {
        self.bridge.abort();
        // Stop the daemon thread so we don't leak it when the service is dropped.
        let _ = self.daemon.shutdown();
    }
}

/// Enable exactly the chosen interfaces on the daemon (and nothing else). Disabling all first
/// makes a re-pin on the same set still re-issue queries — the re-burst the wake path wants.
fn pin_daemon_interfaces(daemon: &ServiceDaemon, chosen: &[Iface]) -> Result<()> {
    daemon
        .disable_interface(IfKind::All)
        .map_err(|e| Error::Sink(format!("mdns disable-all: {e}")))?;
    for iface in chosen {
        daemon
            .enable_interface(IfKind::Addr(iface.ip))
            .map_err(|e| Error::Sink(format!("mdns enable {}: {e}", iface.name)))?;
    }
    Ok(())
}

/// Build one pinned multicast anchor per chosen IPv4 NIC, skipping any that fail to bind (a NIC
/// going away between enumeration and bind is expected churn, not a fatal error — but it is
/// logged, never silently swallowed).
fn build_anchors(chosen: &[Iface]) -> Vec<Socket> {
    chosen
        .iter()
        .filter_map(|iface| match iface.ip {
            IpAddr::V4(v4) => match bind_pinned_multicast_v4(v4) {
                Ok(sock) => Some(sock),
                Err(e) => {
                    tracing::warn!(iface = %iface.name, error = %e, "pinned multicast anchor failed");
                    None
                }
            },
            IpAddr::V6(_) => None,
        })
        .collect()
}

/// Create a UDP multicast socket whose egress is **pinned** to `iface` (`IP_MULTICAST_IF`) and
/// which has **joined** the mDNS group on that same NIC (`IP_ADD_MEMBERSHIP`).
///
/// This is the concrete answer to the tideway black hole: rather than let the route table pick an
/// egress (and lose to a stale `utun`), we name the interface explicitly. The socket binds to an
/// ephemeral port, not 5353, so it never contends with `mdns-sd`'s own sockets; its role is to
/// anchor group membership on the chosen NIC (re-arming switch/host forwarding after a wake) and
/// to be the ready-made egress socket the SSDP source (canon-685a) will send `M-SEARCH` on.
fn bind_pinned_multicast_v4(iface: Ipv4Addr) -> std::io::Result<Socket> {
    let sock = Socket::new(Domain::IPV4, Type::DGRAM, Some(Protocol::UDP))?;
    // SO_REUSEADDR so several multicast listeners can coexist on this host. (SO_REUSEPORT would
    // be nicer on some platforms but needs socket2's "all" feature, which isn't enabled here.)
    sock.set_reuse_address(true)?;
    sock.bind(&SockAddr::from(SocketAddr::from(SocketAddrV4::new(
        Ipv4Addr::UNSPECIFIED,
        0,
    ))))?;
    // Pin egress to the chosen NIC — the actual fix for the stale-route black hole.
    sock.set_multicast_if_v4(&iface)?;
    // Join the mDNS group *on that NIC* so membership is anchored to the right interface.
    sock.join_multicast_v4(&MDNS_GROUP_V4, &iface)?;
    sock.set_nonblocking(true)?;
    Ok(sock)
}

/// Enumerate host interfaces, mapping `if_addrs` into our minimal [`Iface`] at the boundary. On
/// enumeration failure we return empty rather than propagate: no interfaces simply means no LAN
/// discovery this pass, which the resync path can retry.
///
/// Public so callers that must pick a *local* address a renderer can reach — the LAN stream
/// server's bind address, for instance — can feed [`usable_interfaces`] the same host view
/// discovery uses, instead of re-deriving it (and risking a tunnel address).
#[must_use]
pub fn host_interfaces() -> Vec<Iface> {
    if_addrs::get_if_addrs()
        .map(|list| list.iter().map(iface_from).collect())
        .unwrap_or_default()
}

/// The if-addrs → [`Iface`] boundary mapping (kept in one place so nothing else touches if-addrs).
fn iface_from(i: &if_addrs::Interface) -> Iface {
    Iface {
        name: i.name.clone(),
        ip: i.ip(),
        is_loopback: i.is_loopback(),
    }
}

/// Translate an `mdns-sd` event into a cache event, dropping the ones the cache doesn't model
/// (search started/stopped, bare found-before-resolve).
fn to_event(event: ServiceEvent) -> Option<DiscoveryEvent> {
    match event {
        ServiceEvent::ServiceResolved(info) => device_from(&info).map(DiscoveryEvent::Added),
        ServiceEvent::ServiceRemoved(_ty, fullname) => {
            Some(DiscoveryEvent::Removed(SinkId(fullname)))
        }
        _ => None,
    }
}

/// Map a resolved Cast service into a [`DiscoveredDevice`], or `None` if it advertises no usable
/// address. The `SinkId` is the mDNS fullname, which is stable and — crucially — identical
/// between the resolved and removed events, so add and remove line up on the same key.
fn device_from(info: &ResolvedService) -> Option<DiscoveredDevice> {
    let addr = pick_addr(info)?;
    let name = info
        .get_property_val_str("fn")
        .filter(|s| !s.is_empty())
        .map_or_else(
            || instance_label(&info.fullname).to_string(),
            str::to_string,
        );
    Some(DiscoveredDevice {
        id: SinkId(info.fullname.clone()),
        name,
        kind: SinkKind::Chromecast,
        addr,
    })
}

/// Pick a reachable address for a resolved service: prefer IPv4 (Cast speaks it and it avoids
/// scope-id headaches), fall back to IPv6, combine with the advertised port.
fn pick_addr(info: &ResolvedService) -> Option<SocketAddr> {
    let addrs = info.get_addresses();
    let ip = addrs
        .iter()
        .map(|a| a.to_ip_addr())
        .find(IpAddr::is_ipv4)
        .or_else(|| addrs.iter().map(|a| a.to_ip_addr()).find(IpAddr::is_ipv6))?;
    Some(SocketAddr::new(ip, info.get_port()))
}

/// The instance label of an mDNS fullname, e.g. `Living Room` from
/// `Living Room._googlecast._tcp.local.` — the human-facing fallback name.
fn instance_label(fullname: &str) -> &str {
    fullname.split('.').next().unwrap_or(fullname)
}

/// The supervisor loop: fold discovery events, TTL expiry, and resync requests into the engine,
/// and publish a snapshot whenever the debounce fires. Thin by design — all the interesting logic
/// is in the pure components it drives.
async fn supervise(
    mut cast: CastSource,
    mut events: mpsc::Receiver<DiscoveryEvent>,
    mut resync_rx: mpsc::Receiver<()>,
    tx: watch::Sender<Vec<DiscoveredDevice>>,
    ttl: Option<Duration>,
    debounce: Duration,
) {
    let mut engine = SnapshotEngine::new(ttl, debounce);
    // Sweep for lapsed entries at half the TTL — frequent enough to remove a gone device promptly,
    // cheap enough to ignore. With no TTL (the mDNS case, where the source drives removal) there is
    // nothing to sweep, so tick slowly and let `tick_expiry` be a no-op rather than special-casing
    // the select arm.
    let mut expiry =
        tokio::time::interval(ttl.map_or_else(|| Duration::from_secs(3600), |ttl| ttl / 2));

    loop {
        let deadline = engine.deadline();
        tokio::select! {
            maybe_event = events.recv() => match maybe_event {
                Some(event) => engine.apply(event, Instant::now()),
                None => break, // bridge closed; the source is gone.
            },
            _ = expiry.tick() => engine.tick_expiry(Instant::now()),
            maybe_resync = resync_rx.recv() => if let Some(()) = maybe_resync {
                cast.rebuild();
            },
            () = wait_until(deadline) => {}
        }

        if let Some(snapshot) = engine.poll(Instant::now()) {
            // A send error means every picker dropped its receiver; keep the cache warm anyway.
            let _ = tx.send(snapshot);
        }
    }
}

/// Sleep until `deadline`, or forever if there's nothing pending — lets the `select!` treat "a
/// debounce is armed" and "nothing is armed" uniformly.
async fn wait_until(deadline: Option<Instant>) {
    match deadline {
        Some(at) => tokio::time::sleep_until(tokio::time::Instant::from_std(at)).await,
        None => std::future::pending().await,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn iface(name: &str, ip: &str) -> Iface {
        let ip: IpAddr = ip.parse().unwrap();
        Iface {
            name: name.to_string(),
            is_loopback: ip.is_loopback(),
            ip,
        }
    }

    fn dev(id: &str, name: &str, addr: &str) -> DiscoveredDevice {
        DiscoveredDevice {
            id: SinkId(id.to_string()),
            name: name.to_string(),
            kind: SinkKind::Chromecast,
            addr: addr.parse().unwrap(),
        }
    }

    /// Fire the debounce and take the coalesced snapshot; asserts one was actually due.
    fn flush(engine: &mut SnapshotEngine, at: Instant) -> Vec<DiscoveredDevice> {
        engine.poll(at).expect("a snapshot was due")
    }

    /// The crown-jewel filter: tunnels, loopback, and link-local are excluded; only real LAN NICs
    /// survive, in preference order. This is the exact tide-6fd0/8f5b root cause pinned in a test.
    #[test]
    fn usable_interfaces_excludes_tunnels_loopback_and_link_local() {
        let all = vec![
            iface("utun0", "10.0.0.1"),    // VPN tunnel — the black-hole culprit
            iface("tun0", "10.1.0.1"),     // tunnel
            iface("lo0", "127.0.0.1"),     // loopback
            iface("en0", "192.168.1.20"),  // real LAN (Wi-Fi)
            iface("eth0", "10.0.5.4"),     // real LAN (wired)
            iface("en5", "169.254.10.10"), // IPv4 link-local (auto-config, no LAN)
            iface("wg0", "10.9.0.1"),      // WireGuard tunnel
            iface("utun3", "fd00::1"),     // tunnel by name, routable-looking IPv6
            iface("en9", "fe80::1"),       // IPv6 link-local
        ];

        let survivors: Vec<String> = usable_interfaces(&all)
            .into_iter()
            .map(|i| i.name)
            .collect();
        // Only the two real LAN NICs, in their original (preferred) order.
        assert_eq!(survivors, vec!["en0".to_string(), "eth0".to_string()]);
    }

    /// LAN NICs are ordered ahead of surviving oddballs, so egress pins to a real interface.
    #[test]
    fn usable_interfaces_prefers_lan_over_oddballs() {
        let all = vec![
            iface("bridge100", "10.0.0.9"), // valid but non-standard name → kept, but after LAN
            iface("en0", "192.168.1.5"),    // real LAN → must come first
        ];
        let survivors: Vec<String> = usable_interfaces(&all)
            .into_iter()
            .map(|i| i.name)
            .collect();
        assert_eq!(survivors, vec!["en0".to_string(), "bridge100".to_string()]);
    }

    /// The debounced snapshot reflects add, remove, dedupe, and TTL expiry — the anti-flap core.
    #[test]
    fn debounced_snapshot_reflects_add_remove_dedupe_and_expiry() {
        let t0 = Instant::now();
        let ttl = Duration::from_millis(100);
        let debounce = Duration::from_millis(20);
        let mut engine = SnapshotEngine::new(Some(ttl), debounce);

        // Two adds, plus a duplicate add of A (identical content) that must dedupe.
        engine.apply(
            DiscoveryEvent::Added(dev("a", "Alpha", "192.168.1.2:8009")),
            t0,
        );
        engine.apply(
            DiscoveryEvent::Added(dev("b", "Bravo", "192.168.1.3:8009")),
            t0,
        );
        engine.apply(
            DiscoveryEvent::Added(dev("a", "Alpha", "192.168.1.2:8009")),
            t0,
        );
        let ids: Vec<String> = flush(&mut engine, t0 + debounce)
            .into_iter()
            .map(|d| d.id.0)
            .collect();
        assert_eq!(
            ids,
            vec!["a".to_string(), "b".to_string()],
            "both present, A deduped"
        );

        // Explicit remove drops A.
        engine.apply(DiscoveryEvent::Removed(SinkId("a".to_string())), t0);
        let ids: Vec<String> = flush(&mut engine, t0 + debounce)
            .into_iter()
            .map(|d| d.id.0)
            .collect();
        assert_eq!(ids, vec!["b".to_string()], "A removed, B remains");

        // TTL lapses for B (added at t0, ttl 100ms) once we sweep past its deadline.
        let later = t0 + Duration::from_millis(150);
        engine.tick_expiry(later);
        assert!(
            flush(&mut engine, later + debounce).is_empty(),
            "B expired by liveness, not by a missed scan"
        );
    }

    /// A liveness-refreshing re-announcement (same device, same content) must NOT churn the
    /// snapshot — otherwise a device announcing on schedule would publish an update every cycle.
    #[test]
    fn identical_readd_does_not_arm_the_debounce() {
        let t0 = Instant::now();
        let mut engine =
            SnapshotEngine::new(Some(Duration::from_secs(60)), Duration::from_millis(20));

        engine.apply(
            DiscoveryEvent::Added(dev("a", "Alpha", "192.168.1.2:8009")),
            t0,
        );
        let _ = flush(&mut engine, t0 + Duration::from_millis(20));

        // Re-announce the identical device later; nothing changed, so nothing should arm.
        let t1 = t0 + Duration::from_secs(1);
        engine.apply(
            DiscoveryEvent::Added(dev("a", "Alpha", "192.168.1.2:8009")),
            t1,
        );
        assert_eq!(
            engine.deadline(),
            None,
            "unchanged re-announce doesn't arm debounce"
        );
    }

    /// A burst of rapid changes coalesces into exactly one snapshot update.
    #[test]
    fn burst_of_changes_coalesces_into_one_snapshot() {
        let t0 = Instant::now();
        let debounce = Duration::from_millis(50);
        let mut engine = SnapshotEngine::new(Some(Duration::from_secs(60)), debounce);

        let burst = [
            ("a", "Alpha", "10.0.0.2:8009"),
            ("b", "Bravo", "10.0.0.3:8009"),
            ("c", "Charlie", "10.0.0.4:8009"),
        ];
        for (i, (id, name, addr)) in burst.iter().enumerate() {
            let at = t0 + Duration::from_millis(u64::try_from(i).unwrap());
            engine.apply(DiscoveryEvent::Added(dev(id, name, addr)), at);
            assert!(engine.poll(at).is_none(), "nothing fires mid-burst");
        }

        // One update after the window, carrying all three coalesced.
        let snapshot = engine
            .poll(t0 + Duration::from_millis(60))
            .expect("one coalesced update");
        assert_eq!(snapshot.len(), 3);

        // ...and only one: no trailing second update from the same burst.
        assert!(
            engine.poll(t0 + Duration::from_millis(200)).is_none(),
            "burst coalesced into a single update"
        );
    }

    /// Removing something that isn't cached is a no-op that doesn't arm a pointless update.
    #[test]
    fn removing_unknown_device_does_not_arm() {
        let t0 = Instant::now();
        let mut engine =
            SnapshotEngine::new(Some(Duration::from_secs(60)), Duration::from_millis(20));
        engine.apply(DiscoveryEvent::Removed(SinkId("ghost".to_string())), t0);
        assert_eq!(engine.deadline(), None, "removing a ghost changes nothing");
    }

    /// The regression a long-running daemon exposed: with no TTL (the mDNS configuration, where
    /// `mdns-sd` owns liveness and only re-emits on change), a device must stay listed
    /// indefinitely. A blind TTL here reaped live devices after two minutes and nothing re-added
    /// them, because a silently-refreshed device produces no new event to refresh our copy.
    #[test]
    fn without_a_ttl_devices_never_expire() {
        let t0 = Instant::now();
        let mut engine = SnapshotEngine::new(None, Duration::from_millis(20));
        engine.apply(
            DiscoveryEvent::Added(dev("a", "Alpha", "192.168.1.2:8009")),
            t0,
        );
        let listed = flush(&mut engine, t0 + Duration::from_millis(20));
        assert_eq!(listed.len(), 1);

        // Hours later, with no further events at all, the device is still there.
        let much_later = t0 + Duration::from_secs(6 * 3600);
        engine.tick_expiry(much_later);
        assert_eq!(
            engine.deadline(),
            None,
            "expiry must not fire without a TTL"
        );
        assert_eq!(
            engine.cache.snapshot().len(),
            1,
            "a live device must not be reaped by a blind TTL"
        );

        // An explicit remove from the source still works — that is the authoritative signal.
        engine.apply(DiscoveryEvent::Removed(SinkId("a".to_string())), much_later);
        let after = flush(&mut engine, much_later + Duration::from_millis(20));
        assert_eq!(after.len(), 0);
    }
}
