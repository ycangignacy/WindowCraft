// Made by ycangignacy
//! Parallel DNS benchmark measuring real query latency over UDP, TCP and DoH.

use crate::download::http_client;
use crate::error::{EngineError, Result};
use std::net::{IpAddr, SocketAddr};
use std::time::{Duration, Instant};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpStream, UdpSocket};
use tokio::time::timeout;
use windowcraft_config::DnsProvider;

const QUERY_TIMEOUT: Duration = Duration::from_secs(2);
/// A protocol that never answers scores as a full timeout. Skipping it would
/// reward a resolver for failing its slowest protocol.
const FAILED_PROTOCOL_MS: f64 = 2_000.0;
const DOMAINS: [&str; 5] = ["example.com", "www.microsoft.com", "www.wikipedia.org", "github.com", "cloudflare.com"];

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Protocol {
    Udp,
    Tcp,
    Doh,
}

impl Protocol {
    pub fn label(self) -> &'static str {
        match self {
            Self::Udp => "UDP",
            Self::Tcp => "TCP",
            Self::Doh => "DoH",
        }
    }
}

#[derive(Debug, Clone)]
pub struct Target {
    pub id: String,
    pub udp: SocketAddr,
    pub tcp: SocketAddr,
    pub doh: Option<String>,
}

impl Target {
    pub fn from_provider(provider: &DnsProvider) -> Option<Self> {
        let address: IpAddr = provider.primary.parse().ok()?;
        let socket = SocketAddr::new(address, 53);
        Some(Self { id: provider.id.clone(), udp: socket, tcp: socket, doh: Some(provider.doh_template.clone()) })
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct Measurement {
    pub protocol: Protocol,
    pub min_ms: f64,
    pub average_ms: f64,
    pub jitter_ms: f64,
    pub failures: u32,
}

#[derive(Debug, Clone, PartialEq)]
pub struct BenchResult {
    pub id: String,
    pub measurements: Vec<Measurement>,
}

impl BenchResult {
    /// Average across every measured protocol, or `None` if none answered.
    pub fn score(&self) -> Option<f64> {
        if !self.measurements.iter().any(|m| m.average_ms.is_finite()) {
            return None;
        }
        let total: f64 = self
            .measurements
            .iter()
            .map(|m| if m.average_ms.is_finite() { m.average_ms } else { FAILED_PROTOCOL_MS })
            .sum();
        Some(total / self.measurements.len() as f64)
    }
}

/// Builds a minimal A-record query (RFC 1035).
pub fn build_query(id: u16, name: &str) -> Vec<u8> {
    let mut packet = Vec::with_capacity(name.len() + 18);
    packet.extend_from_slice(&id.to_be_bytes());
    packet.extend_from_slice(&[0x01, 0x00, 0, 1, 0, 0, 0, 0, 0, 0]);
    for label in name.split('.') {
        packet.push(label.len().min(63) as u8);
        packet.extend_from_slice(&label.as_bytes()[..label.len().min(63)]);
    }
    packet.push(0);
    packet.extend_from_slice(&[0, 1, 0, 1]);
    packet
}

/// A response counts when the id matches, the QR bit is set and it is not a
/// server failure. NXDOMAIN still measures a complete round trip.
pub fn response_ok(id: u16, response: &[u8]) -> bool {
    response.len() >= 12
        && response[..2] == id.to_be_bytes()
        && response[2] & 0x80 != 0
        && !matches!(response[3] & 0x0F, 2 | 5)
}

async fn query_udp(server: SocketAddr, id: u16, name: &str) -> Result<()> {
    let bind: SocketAddr = if server.is_ipv4() { ([0, 0, 0, 0], 0).into() } else { ([0u16; 8], 0).into() };
    let socket = UdpSocket::bind(bind).await.map_err(|e| EngineError::Network(e.to_string()))?;
    socket.connect(server).await.map_err(|e| EngineError::Network(e.to_string()))?;
    socket.send(&build_query(id, name)).await.map_err(|e| EngineError::Network(e.to_string()))?;
    let mut buffer = [0u8; 1500];
    let size = socket.recv(&mut buffer).await.map_err(|e| EngineError::Network(e.to_string()))?;
    if response_ok(id, &buffer[..size]) { Ok(()) } else { Err(EngineError::Network("bad response".to_owned())) }
}

async fn query_tcp(server: SocketAddr, id: u16, name: &str) -> Result<()> {
    let mut stream = TcpStream::connect(server).await.map_err(|e| EngineError::Network(e.to_string()))?;
    let packet = build_query(id, name);
    let mut framed = (packet.len() as u16).to_be_bytes().to_vec();
    framed.extend_from_slice(&packet);
    stream.write_all(&framed).await.map_err(|e| EngineError::Network(e.to_string()))?;
    let length = stream.read_u16().await.map_err(|e| EngineError::Network(e.to_string()))?;
    let mut buffer = vec![0u8; usize::from(length)];
    stream.read_exact(&mut buffer).await.map_err(|e| EngineError::Network(e.to_string()))?;
    if response_ok(id, &buffer) { Ok(()) } else { Err(EngineError::Network("bad response".to_owned())) }
}

async fn query_doh(client: &reqwest::Client, url: &str, id: u16, name: &str) -> Result<()> {
    // RFC 8484 recommends id 0 for cache friendliness; a cache hit would hide
    // resolver latency here, so the real id stays in.
    let response = client
        .post(url)
        .header("content-type", "application/dns-message")
        .header("accept", "application/dns-message")
        .body(build_query(id, name))
        .send()
        .await
        .map_err(|e| EngineError::Network(e.to_string()))?;
    let bytes = response.bytes().await.map_err(|e| EngineError::Network(e.to_string()))?;
    if response_ok(id, &bytes) { Ok(()) } else { Err(EngineError::Network("bad response".to_owned())) }
}

/// Computes min, average and jitter (mean absolute successive difference).
pub fn summarize(protocol: Protocol, samples: &[f64], failures: u32) -> Measurement {
    if samples.is_empty() {
        return Measurement { protocol, min_ms: f64::NAN, average_ms: f64::NAN, jitter_ms: f64::NAN, failures };
    }
    let min_ms = samples.iter().copied().fold(f64::INFINITY, f64::min);
    let average_ms = samples.iter().sum::<f64>() / samples.len() as f64;
    let jitter_ms = if samples.len() < 2 {
        0.0
    } else {
        samples.windows(2).map(|pair| (pair[1] - pair[0]).abs()).sum::<f64>() / (samples.len() - 1) as f64
    };
    Measurement { protocol, min_ms, average_ms, jitter_ms, failures }
}

async fn measure(target: &Target, protocol: Protocol, client: Option<&reqwest::Client>, rounds: u16) -> Measurement {
    let mut samples = Vec::with_capacity(usize::from(rounds));
    let mut failures = 0;
    for round in 0..rounds {
        let id = 0x5700 ^ round;
        let name = DOMAINS[usize::from(round) % DOMAINS.len()];
        let started = Instant::now();
        let outcome = match protocol {
            Protocol::Udp => timeout(QUERY_TIMEOUT, query_udp(target.udp, id, name)).await,
            Protocol::Tcp => timeout(QUERY_TIMEOUT, query_tcp(target.tcp, id, name)).await,
            Protocol::Doh => match (client, target.doh.as_deref()) {
                (Some(client), Some(url)) => timeout(QUERY_TIMEOUT, query_doh(client, url, id, name)).await,
                _ => Ok(Err(EngineError::Network("no DoH endpoint".to_owned()))),
            },
        };
        match outcome {
            Ok(Ok(())) => samples.push(started.elapsed().as_secs_f64() * 1_000.0),
            _ => failures += 1,
        }
    }
    summarize(protocol, &samples, failures)
}

async fn bench_target(target: Target, client: Option<reqwest::Client>, rounds: u16) -> BenchResult {
    let (udp, tcp, doh) = tokio::join!(
        measure(&target, Protocol::Udp, None, rounds),
        measure(&target, Protocol::Tcp, None, rounds),
        async {
            // A target with no DoH endpoint has nothing to measure or fail.
            match target.doh {
                Some(_) => Some(measure(&target, Protocol::Doh, client.as_ref(), rounds).await),
                None => None,
            }
        },
    );
    let mut measurements = vec![udp, tcp];
    measurements.extend(doh);
    BenchResult { id: target.id, measurements }
}

/// Benchmarks every target concurrently, fastest first.
pub async fn benchmark(targets: Vec<Target>, rounds: u16) -> Result<Vec<BenchResult>> {
    let client = if targets.iter().any(|target| target.doh.is_some()) { Some(http_client()?) } else { None };
    let mut tasks = Vec::with_capacity(targets.len());
    for target in targets {
        tasks.push(tokio::spawn(bench_target(target, client.clone(), rounds)));
    }
    let mut results = Vec::with_capacity(tasks.len());
    for task in tasks {
        results.push(task.await.map_err(|e| EngineError::Network(e.to_string()))?);
    }
    results.sort_by(|a, b| {
        let (a, b) = (a.score().unwrap_or(f64::MAX), b.score().unwrap_or(f64::MAX));
        a.total_cmp(&b)
    });
    Ok(results)
}

/// The provider a `Fastest` DNS selection resolves to.
pub fn fastest(results: &[BenchResult]) -> Option<&str> {
    results.iter().find(|result| result.score().is_some()).map(|result| result.id.as_str())
}

#[cfg(test)]
mod tests {
    use super::*;
    use tokio::net::TcpListener;

    #[test]
    fn query_encoding() {
        let packet = build_query(0x1234, "a.bc");
        assert_eq!(&packet[..2], &[0x12, 0x34]);
        assert_eq!(&packet[12..18], &[1, b'a', 2, b'b', b'c', 0]);
        assert_eq!(&packet[18..], &[0, 1, 0, 1]);
    }

    #[test]
    fn response_validation() {
        let mut response = vec![0x12, 0x34, 0x81, 0x80, 0, 1, 0, 0, 0, 0, 0, 0];
        assert!(response_ok(0x1234, &response));
        assert!(!response_ok(0x4321, &response), "mismatched id");
        response[3] = 0x82;
        assert!(!response_ok(0x1234, &response), "SERVFAIL");
        response[3] = 0x83;
        assert!(response_ok(0x1234, &response), "NXDOMAIN is a real answer");
        response[2] = 0x01;
        assert!(!response_ok(0x1234, &response), "not a response");
        assert!(!response_ok(0x1234, &[1, 2]), "truncated");
    }

    #[test]
    fn statistics() {
        let result = summarize(Protocol::Udp, &[10.0, 20.0, 15.0], 1);
        assert_eq!(result.min_ms, 10.0);
        assert_eq!(result.average_ms, 15.0);
        assert_eq!(result.jitter_ms, 7.5);
        assert_eq!(result.failures, 1);
        assert!(summarize(Protocol::Tcp, &[], 3).average_ms.is_nan());
        assert_eq!(summarize(Protocol::Tcp, &[5.0], 0).jitter_ms, 0.0);
    }

    fn answer(query: &[u8]) -> Vec<u8> {
        let mut response = query.to_vec();
        response[2] = 0x81;
        response[3] = 0x80;
        response
    }

    async fn mock_server() -> (SocketAddr, SocketAddr) {
        let udp = UdpSocket::bind(("127.0.0.1", 0)).await.unwrap();
        let tcp = TcpListener::bind(("127.0.0.1", 0)).await.unwrap();
        let addresses = (udp.local_addr().unwrap(), tcp.local_addr().unwrap());
        tokio::spawn(async move {
            let mut buffer = [0u8; 512];
            while let Ok((size, peer)) = udp.recv_from(&mut buffer).await {
                let _ = udp.send_to(&answer(&buffer[..size]), peer).await;
            }
        });
        tokio::spawn(async move {
            while let Ok((mut stream, _)) = tcp.accept().await {
                tokio::spawn(async move {
                    let Ok(length) = stream.read_u16().await else { return };
                    let mut query = vec![0u8; usize::from(length)];
                    if stream.read_exact(&mut query).await.is_ok() {
                        let reply = answer(&query);
                        let mut framed = (reply.len() as u16).to_be_bytes().to_vec();
                        framed.extend_from_slice(&reply);
                        let _ = stream.write_all(&framed).await;
                    }
                });
            }
        });
        addresses
    }

    #[tokio::test]
    async fn benchmarks_targets_concurrently_against_a_local_server() {
        let (udp, tcp) = mock_server().await;
        let targets: Vec<Target> =
            (0..4).map(|i| Target { id: format!("mock{i}"), udp, tcp, doh: None }).collect();
        let results = benchmark(targets, 4).await.unwrap();
        assert_eq!(results.len(), 4);
        for result in &results {
            assert!(result.score().unwrap() < 500.0);
            assert_eq!(result.measurements[0].failures, 0);
            assert_eq!(result.measurements[1].failures, 0);
            assert_eq!(result.measurements.len(), 2, "no DoH endpoint configured");
        }
        assert!(fastest(&results).is_some());
    }

    #[test]
    fn a_failed_protocol_cannot_make_a_resolver_look_faster() {
        let fast_but_broken = BenchResult {
            id: "broken".into(),
            measurements: vec![summarize(Protocol::Udp, &[10.0], 0), summarize(Protocol::Doh, &[], 6)],
        };
        let healthy = BenchResult {
            id: "healthy".into(),
            measurements: vec![summarize(Protocol::Udp, &[20.0], 0), summarize(Protocol::Doh, &[60.0], 0)],
        };
        assert!(healthy.score().unwrap() < fast_but_broken.score().unwrap());
    }

    #[tokio::test]
    async fn unreachable_servers_count_as_failures() {
        let dead: SocketAddr = ([127, 0, 0, 1], 9).into();
        let target = Target { id: "dead".into(), udp: dead, tcp: dead, doh: None };
        let result = bench_target(target, None, 1).await;
        assert!(result.measurements.iter().all(|m| m.failures == 1));
        assert_eq!(result.score(), None);
        assert_eq!(fastest(&[result]), None);
    }
}
