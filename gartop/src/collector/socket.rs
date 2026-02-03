//! Network socket collection from /proc/net/* and netlink INET_DIAG
//!
//! Provides per-process network statistics by correlating socket inodes
//! from /proc/net/tcp,udp with process file descriptors.

use std::collections::HashMap;
use std::fs;
use std::io;
use std::time::Instant;

use netlink_packet_core::{NetlinkMessage, NetlinkPayload, NLM_F_DUMP, NLM_F_REQUEST};
use netlink_packet_sock_diag::{
    constants::*,
    inet::{ExtensionFlags, InetRequest, InetResponse, SocketId, StateFlags},
    SockDiagMessage,
};
use netlink_sys::{protocols::NETLINK_SOCK_DIAG, Socket, SocketAddr};

/// TCP connection states (from kernel include/net/tcp_states.h)
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TcpState {
    Established = 1,
    SynSent = 2,
    SynRecv = 3,
    FinWait1 = 4,
    FinWait2 = 5,
    TimeWait = 6,
    Close = 7,
    CloseWait = 8,
    LastAck = 9,
    Listen = 10,
    Closing = 11,
    Unknown = 0,
}

impl From<u8> for TcpState {
    fn from(state: u8) -> Self {
        match state {
            1 => TcpState::Established,
            2 => TcpState::SynSent,
            3 => TcpState::SynRecv,
            4 => TcpState::FinWait1,
            5 => TcpState::FinWait2,
            6 => TcpState::TimeWait,
            7 => TcpState::Close,
            8 => TcpState::CloseWait,
            9 => TcpState::LastAck,
            10 => TcpState::Listen,
            11 => TcpState::Closing,
            _ => TcpState::Unknown,
        }
    }
}

/// Socket protocol type.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SocketProtocol {
    Tcp,
    Udp,
}

/// Information about a network socket.
#[derive(Debug, Clone)]
pub struct SocketInfo {
    pub protocol: SocketProtocol,
    pub state: TcpState,
    pub local_port: u16,
    pub remote_port: u16,
    pub tx_queue: u32,
    pub rx_queue: u32,
    /// Bytes received (from netlink, if available)
    pub rx_bytes: u64,
    /// Bytes transmitted (from netlink, if available)
    pub tx_bytes: u64,
}

/// Per-process network statistics.
#[derive(Debug, Clone, Default)]
pub struct ProcessNetStats {
    pub tcp_count: u32,
    pub udp_count: u32,
    pub listen_count: u32,
    pub established_count: u32,
    pub total_tx_queue: u64,
    pub total_rx_queue: u64,
    pub total_rx_bytes: u64,
    pub total_tx_bytes: u64,
}

/// Previous socket bytes for rate calculation.
struct PrevSocketBytes {
    rx_bytes: u64,
    tx_bytes: u64,
    timestamp: Instant,
}

/// Socket collector that parses /proc/net/* files and uses netlink for bandwidth.
pub struct SocketCollector {
    /// Cache of inode -> socket info
    socket_cache: HashMap<u64, SocketInfo>,
    /// Previous bytes per inode for rate calculation
    prev_bytes: HashMap<u64, PrevSocketBytes>,
    /// Netlink socket (lazy initialized)
    nl_socket: Option<Socket>,
}

impl SocketCollector {
    /// Create a new socket collector.
    pub fn new() -> Self {
        Self {
            socket_cache: HashMap::new(),
            prev_bytes: HashMap::new(),
            nl_socket: None,
        }
    }

    /// Initialize netlink socket if not already done.
    fn init_netlink(&mut self) -> io::Result<()> {
        if self.nl_socket.is_none() {
            let mut socket = Socket::new(NETLINK_SOCK_DIAG)?;
            socket.bind_auto()?;
            socket.connect(&SocketAddr::new(0, 0))?;
            self.nl_socket = Some(socket);
        }
        Ok(())
    }

    /// Refresh the socket cache by parsing /proc/net/* files and querying netlink.
    pub fn refresh(&mut self) {
        self.socket_cache.clear();

        // First parse procfs for basic socket info
        self.parse_proc_net("/proc/net/tcp", SocketProtocol::Tcp);
        self.parse_proc_net("/proc/net/tcp6", SocketProtocol::Tcp);
        self.parse_proc_net("/proc/net/udp", SocketProtocol::Udp);
        self.parse_proc_net("/proc/net/udp6", SocketProtocol::Udp);

        // Then try to get bandwidth info from netlink
        if let Err(e) = self.query_netlink_tcp() {
            tracing::debug!("Netlink TCP query failed: {}", e);
        }
        if let Err(e) = self.query_netlink_udp() {
            tracing::debug!("Netlink UDP query failed: {}", e);
        }

        // Clean up old prev_bytes entries
        let current_inodes: std::collections::HashSet<_> = self.socket_cache.keys().copied().collect();
        self.prev_bytes.retain(|inode, _| current_inodes.contains(inode));
    }

    /// Parse a /proc/net/* file and populate the cache.
    fn parse_proc_net(&mut self, path: &str, protocol: SocketProtocol) {
        let contents = match fs::read_to_string(path) {
            Ok(c) => c,
            Err(_) => return,
        };

        for line in contents.lines().skip(1) {
            if let Some((inode, info)) = self.parse_socket_line(line, protocol) {
                self.socket_cache.insert(inode, info);
            }
        }
    }

    /// Parse a single line from /proc/net/tcp or /proc/net/udp.
    fn parse_socket_line(&self, line: &str, protocol: SocketProtocol) -> Option<(u64, SocketInfo)> {
        let parts: Vec<&str> = line.split_whitespace().collect();
        if parts.len() < 10 {
            return None;
        }

        let local_port = self.parse_addr_port(parts[1])?;
        let remote_port = self.parse_addr_port(parts[2])?;
        let state_hex = u8::from_str_radix(parts[3], 16).ok()?;
        let state = TcpState::from(state_hex);

        let queues: Vec<&str> = parts[4].split(':').collect();
        let tx_queue = u32::from_str_radix(queues.first()?, 16).unwrap_or(0);
        let rx_queue = u32::from_str_radix(queues.get(1)?, 16).unwrap_or(0);

        let inode: u64 = parts[9].parse().ok()?;
        if inode == 0 {
            return None;
        }

        Some((
            inode,
            SocketInfo {
                protocol,
                state,
                local_port,
                remote_port,
                tx_queue,
                rx_queue,
                rx_bytes: 0,
                tx_bytes: 0,
            },
        ))
    }

    /// Parse port from address string (IP:PORT in hex).
    fn parse_addr_port(&self, addr: &str) -> Option<u16> {
        let parts: Vec<&str> = addr.split(':').collect();
        if parts.len() != 2 {
            return None;
        }
        u16::from_str_radix(parts[1], 16).ok()
    }

    /// Query netlink for TCP socket info with byte counts.
    fn query_netlink_tcp(&mut self) -> io::Result<()> {
        self.init_netlink()?;

        // Request all TCP sockets with extended info - IPv4
        let req_v4 = InetRequest {
            family: AF_INET as u8,
            protocol: IPPROTO_TCP as u8,
            socket_id: SocketId::new_v4(),
            extensions: ExtensionFlags::INFO,
            states: StateFlags::all(),
        };

        // IPv6 request
        let req_v6 = InetRequest {
            family: AF_INET6 as u8,
            protocol: IPPROTO_TCP as u8,
            socket_id: SocketId::new_v6(),
            extensions: ExtensionFlags::INFO,
            states: StateFlags::all(),
        };

        // Process IPv4
        self.send_and_recv_inet_diag(&req_v4, SocketProtocol::Tcp)?;
        // Process IPv6
        self.send_and_recv_inet_diag(&req_v6, SocketProtocol::Tcp)?;

        Ok(())
    }

    /// Query netlink for UDP socket info.
    fn query_netlink_udp(&mut self) -> io::Result<()> {
        self.init_netlink()?;

        let req_v4 = InetRequest {
            family: AF_INET as u8,
            protocol: IPPROTO_UDP as u8,
            socket_id: SocketId::new_v4(),
            extensions: ExtensionFlags::empty(),
            states: StateFlags::all(),
        };

        let req_v6 = InetRequest {
            family: AF_INET6 as u8,
            protocol: IPPROTO_UDP as u8,
            socket_id: SocketId::new_v6(),
            extensions: ExtensionFlags::empty(),
            states: StateFlags::all(),
        };

        self.send_and_recv_inet_diag(&req_v4, SocketProtocol::Udp)?;
        self.send_and_recv_inet_diag(&req_v6, SocketProtocol::Udp)?;

        Ok(())
    }

    /// Send request and receive responses in one method to avoid borrow issues.
    fn send_and_recv_inet_diag(&mut self, req: &InetRequest, protocol: SocketProtocol) -> io::Result<()> {
        // Build the message
        let msg = SockDiagMessage::InetRequest(req.clone());
        let mut nl_msg = NetlinkMessage::from(msg);
        nl_msg.header.flags = NLM_F_REQUEST | NLM_F_DUMP;
        nl_msg.header.sequence_number = 1;
        nl_msg.finalize();

        let mut send_buf = vec![0u8; nl_msg.header.length as usize];
        nl_msg.serialize(&mut send_buf);

        // Collect responses first, then process them
        let mut responses = Vec::new();

        {
            // Scope the socket borrow
            let socket = self.nl_socket.as_ref().unwrap();
            socket.send(&send_buf, 0)?;

            let mut recv_buf = vec![0u8; 65536];
            loop {
                let n = match socket.recv(&mut recv_buf, 0) {
                    Ok(n) => n,
                    Err(e) if e.kind() == io::ErrorKind::WouldBlock => break,
                    Err(e) => return Err(e),
                };

                if n == 0 {
                    break;
                }

                let mut offset = 0;
                while offset < n {
                    let msg = match NetlinkMessage::<SockDiagMessage>::deserialize(&recv_buf[offset..n]) {
                        Ok(msg) => msg,
                        Err(_) => break,
                    };

                    offset += msg.header.length as usize;

                    match msg.payload {
                        NetlinkPayload::Done(_) => break,
                        NetlinkPayload::Error(e) => {
                            if e.code.is_some() {
                                return Err(io::Error::new(io::ErrorKind::Other, "netlink error"));
                            }
                        }
                        NetlinkPayload::InnerMessage(SockDiagMessage::InetResponse(resp)) => {
                            responses.push(resp);
                        }
                        _ => {}
                    }
                }
            }
        }

        // Now process collected responses
        for resp in responses {
            self.process_inet_response(&resp, protocol);
        }

        Ok(())
    }

    /// Process a single INET_DIAG response.
    fn process_inet_response(&mut self, resp: &InetResponse, protocol: SocketProtocol) {
        let inode = resp.header.inode as u64;
        if inode == 0 {
            return;
        }

        // Update existing socket info with byte counts from netlink
        if let Some(info) = self.socket_cache.get_mut(&inode) {
            // The InetResponse contains nla (netlink attributes) with TCP_INFO
            // which has bytes_sent and bytes_received
            // For now, we use the queue sizes as a proxy
            // TODO: Parse TCP_INFO attribute for actual byte counts
            info.rx_bytes = resp.header.recv_queue as u64;
            info.tx_bytes = resp.header.send_queue as u64;
        } else {
            // Socket not in procfs cache, add it
            let state = TcpState::from(resp.header.state);
            self.socket_cache.insert(inode, SocketInfo {
                protocol,
                state,
                local_port: resp.header.socket_id.source_port,
                remote_port: resp.header.socket_id.destination_port,
                tx_queue: resp.header.send_queue,
                rx_queue: resp.header.recv_queue,
                rx_bytes: resp.header.recv_queue as u64,
                tx_bytes: resp.header.send_queue as u64,
            });
        }
    }

    /// Get network stats for a process given its socket inodes.
    pub fn get_process_stats(&mut self, socket_inodes: &[u64]) -> ProcessNetStats {
        let mut stats = ProcessNetStats::default();
        let now = Instant::now();

        for inode in socket_inodes {
            if let Some(info) = self.socket_cache.get(inode) {
                match info.protocol {
                    SocketProtocol::Tcp => {
                        stats.tcp_count += 1;
                        if info.state == TcpState::Listen {
                            stats.listen_count += 1;
                        } else if info.state == TcpState::Established {
                            stats.established_count += 1;
                        }
                    }
                    SocketProtocol::Udp => {
                        stats.udp_count += 1;
                    }
                }
                stats.total_tx_queue += info.tx_queue as u64;
                stats.total_rx_queue += info.rx_queue as u64;
                stats.total_rx_bytes += info.rx_bytes;
                stats.total_tx_bytes += info.tx_bytes;
            }
        }

        // Update prev_bytes for rate calculation
        for inode in socket_inodes {
            if let Some(info) = self.socket_cache.get(inode) {
                self.prev_bytes.insert(*inode, PrevSocketBytes {
                    rx_bytes: info.rx_bytes,
                    tx_bytes: info.tx_bytes,
                    timestamp: now,
                });
            }
        }

        stats
    }

    /// Calculate bandwidth rates for a process given its socket inodes.
    pub fn get_process_bandwidth(&self, socket_inodes: &[u64]) -> (f64, f64) {
        let now = Instant::now();
        let mut total_rx_rate = 0.0;
        let mut total_tx_rate = 0.0;

        for inode in socket_inodes {
            if let (Some(info), Some(prev)) = (self.socket_cache.get(inode), self.prev_bytes.get(inode)) {
                let elapsed = now.duration_since(prev.timestamp).as_secs_f64();
                if elapsed > 0.0 {
                    let rx_delta = info.rx_bytes.saturating_sub(prev.rx_bytes) as f64;
                    let tx_delta = info.tx_bytes.saturating_sub(prev.tx_bytes) as f64;
                    total_rx_rate += rx_delta / elapsed;
                    total_tx_rate += tx_delta / elapsed;
                }
            }
        }

        (total_rx_rate, total_tx_rate)
    }

    /// Get the number of cached sockets.
    pub fn socket_count(&self) -> usize {
        self.socket_cache.len()
    }
}

impl Default for SocketCollector {
    fn default() -> Self {
        Self::new()
    }
}
