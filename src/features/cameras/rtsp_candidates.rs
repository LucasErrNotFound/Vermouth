use std::{
    io::{Read, Write},
    net::{Ipv4Addr, SocketAddrV4, TcpStream},
    time::{Duration, Instant},
};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum StreamQuality {
    Main,
    Sub,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ProbeResult {
    VideoSdpAccepted,
    AuthenticationRequired,
    Rejected,
    Indeterminate,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Candidate {
    pub url: String,
    pub quality: StreamQuality,
    pub result: ProbeResult,
}

pub fn probe_v380(
    ip: Ipv4Addr,
    open_ports: &[u16],
    cancelled: &std::sync::atomic::AtomicBool,
) -> Vec<Candidate> {
    use std::sync::atomic::Ordering;

    let ports: Vec<u16> = [554, 8554]
        .into_iter()
        .filter(|port| open_ports.contains(port))
        .collect();
    let ports = if ports.is_empty() { vec![554] } else { ports };
    let mut candidates = Vec::new();

    for (quality, path) in [
        (StreamQuality::Main, "/live/ch00_1"),
        (StreamQuality::Sub, "/live/ch00_0"),
    ] {
        for &port in &ports {
            if cancelled.load(Ordering::Relaxed) {
                return candidates;
            }
            let url = format!("rtsp://{ip}:{port}{path}");
            let result = probe(ip, port, &url, Duration::from_millis(450));
            candidates.push(Candidate {
                url,
                quality,
                result,
            });
        }
    }
    candidates
}

pub fn preferred(candidates: &[Candidate]) -> Option<&Candidate> {
    candidates
        .iter()
        .find(|item| {
            item.quality == StreamQuality::Main && item.result == ProbeResult::VideoSdpAccepted
        })
        .or_else(|| {
            candidates
                .iter()
                .find(|item| item.result == ProbeResult::VideoSdpAccepted)
        })
        .or_else(|| {
            candidates
                .iter()
                .find(|item| item.result == ProbeResult::AuthenticationRequired)
        })
        .or_else(|| {
            candidates
                .iter()
                .find(|item| item.quality == StreamQuality::Main)
        })
}

fn probe(ip: Ipv4Addr, port: u16, url: &str, timeout: Duration) -> ProbeResult {
    let address: std::net::SocketAddr = SocketAddrV4::new(ip, port).into();
    let Ok(mut stream) = TcpStream::connect_timeout(&address, timeout) else {
        return ProbeResult::Indeterminate;
    };
    if stream
        .set_read_timeout(Some(Duration::from_millis(100)))
        .is_err()
        || stream.set_write_timeout(Some(timeout)).is_err()
    {
        return ProbeResult::Indeterminate;
    }

    let request = format!(
        "DESCRIBE {url} RTSP/1.0\r\nCSeq: 1\r\nAccept: application/sdp\r\nUser-Agent: Vermouth\r\n\r\n"
    );
    if stream.write_all(request.as_bytes()).is_err() {
        return ProbeResult::Indeterminate;
    }

    let deadline = Instant::now() + timeout;
    let mut response = Vec::with_capacity(4096);
    let mut buffer = [0u8; 2048];
    while Instant::now() < deadline && response.len() < 16 * 1024 {
        match stream.read(&mut buffer) {
            Ok(0) => break,
            Ok(length) => response.extend_from_slice(&buffer[..length]),
            Err(error)
                if matches!(
                    error.kind(),
                    std::io::ErrorKind::WouldBlock | std::io::ErrorKind::TimedOut
                ) =>
            {
                if !response.is_empty() {
                    break;
                }
            }
            Err(_) => break,
        }
    }
    classify_response(&response)
}

fn classify_response(bytes: &[u8]) -> ProbeResult {
    let Ok(response) = std::str::from_utf8(bytes) else {
        return ProbeResult::Indeterminate;
    };
    let Some(status) = response.lines().next() else {
        return ProbeResult::Indeterminate;
    };
    let mut words = status.split_whitespace();
    if words.next() != Some("RTSP/1.0") {
        return ProbeResult::Indeterminate;
    }
    match words.next().and_then(|code| code.parse::<u16>().ok()) {
        Some(200) => {
            if response.contains("m=video") {
                ProbeResult::VideoSdpAccepted
            } else {
                ProbeResult::Indeterminate
            }
        }
        Some(401) => ProbeResult::AuthenticationRequired,
        Some(404 | 400 | 405 | 454) => ProbeResult::Rejected,
        Some(_) | None => ProbeResult::Indeterminate,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn recognizes_video_sdp_not_just_http_or_rtsp_200() {
        assert_eq!(
            classify_response(b"RTSP/1.0 200 OK\r\nContent-Type: application/sdp\r\n\r\nv=0\r\nm=video 0 RTP/AVP 96\r\n"),
            ProbeResult::VideoSdpAccepted
        );
        assert_eq!(
            classify_response(b"RTSP/1.0 200 OK\r\n\r\n"),
            ProbeResult::Indeterminate
        );
        assert_eq!(
            classify_response(b"RTSP/1.0 401 Unauthorized\r\n\r\n"),
            ProbeResult::AuthenticationRequired
        );
    }

    #[test]
    fn prefers_confirmed_main_stream_over_unverified_main() {
        let candidates = vec![
            Candidate {
                url: "main".into(),
                quality: StreamQuality::Main,
                result: ProbeResult::Indeterminate,
            },
            Candidate {
                url: "sub".into(),
                quality: StreamQuality::Sub,
                result: ProbeResult::VideoSdpAccepted,
            },
        ];
        assert_eq!(preferred(&candidates).unwrap().url, "sub");
    }
}
