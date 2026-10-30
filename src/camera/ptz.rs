use std::{
    io::{Read, Write},
    net::{Ipv4Addr, SocketAddrV4, TcpStream},
    sync::mpsc::{self, Receiver, RecvTimeoutError, Sender},
    thread,
    time::{Duration, Instant},
};

use roxmltree::Document;

use super::CameraId;

const IO_TIMEOUT: Duration = Duration::from_millis(650);
const MAX_REPLY: usize = 128 * 1024;
const MOVE_REFRESH: Duration = Duration::from_millis(250);
const KEEPALIVE_DEADLINE: Duration = Duration::from_millis(450);
const CAMERA_MOVE_TIMEOUT: &str = "PT0.8S";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Direction {
    Up,
    Down,
    Left,
    Right,
}

impl Direction {
    fn velocity(self) -> (&'static str, &'static str) {
        match self {
            Self::Up => ("0", "0.35"),
            Self::Down => ("0", "-0.35"),
            Self::Left => ("-0.35", "0"),
            Self::Right => ("0.35", "0"),
        }
    }
}

#[derive(Clone, Debug)]
pub struct Target {
    endpoint: Endpoint,
    profile: String,
}

#[derive(Clone, Debug)]
struct Endpoint {
    ip: Ipv4Addr,
    port: u16,
    path: String,
}

#[derive(Debug)]
pub enum Event {
    Discovered(CameraId, String, Result<Target, String>),
    CommandFailed(CameraId, String),
}

pub enum Command {
    Discover(CameraId, String),
    Start(CameraId, Target, Direction),
    KeepAlive(CameraId),
    Stop(CameraId),
    Shutdown,
}

struct Motion {
    id: CameraId,
    target: Target,
    direction: Direction,
    refreshed: Instant,
    heartbeat: Instant,
}

pub fn spawn_worker() -> (Sender<Command>, Receiver<Event>) {
    let (commands_tx, commands_rx) = mpsc::channel();
    let (events_tx, events_rx) = mpsc::channel();
    thread::spawn(move || worker(commands_rx, events_tx));
    (commands_tx, events_rx)
}

fn worker(rx: Receiver<Command>, events: Sender<Event>) {
    let mut active: Option<Motion> = None;
    loop {
        match rx.recv_timeout(Duration::from_millis(70)) {
            Ok(Command::Discover(id, url)) => {
                stop_motion(&mut active);
                let result = discover(&url);
                let _ = events.send(Event::Discovered(id, url, result));
            }
            Ok(Command::Start(id, target, direction)) => {
                stop_motion(&mut active);
                match move_camera(&target, direction) {
                    Ok(()) => {
                        let now = Instant::now();
                        active = Some(Motion {
                            id,
                            target,
                            direction,
                            refreshed: now,
                            heartbeat: now,
                        });
                    }
                    Err(error) => {
                        let _ = events.send(Event::CommandFailed(id, error));
                    }
                }
            }
            Ok(Command::KeepAlive(id)) => {
                if let Some(motion) = active.as_mut().filter(|motion| motion.id == id) {
                    motion.heartbeat = Instant::now();
                }
            }
            Ok(Command::Stop(id)) => {
                if active.as_ref().is_some_and(|motion| motion.id == id) {
                    stop_motion(&mut active);
                }
            }
            Ok(Command::Shutdown) | Err(RecvTimeoutError::Disconnected) => {
                stop_motion(&mut active);
                break;
            }
            Err(RecvTimeoutError::Timeout) => {}
        }

        if let Some(motion) = active.as_mut() {
            let now = Instant::now();
            if now.duration_since(motion.heartbeat) >= KEEPALIVE_DEADLINE {
                stop_motion(&mut active);
            } else if now.duration_since(motion.refreshed) >= MOVE_REFRESH {
                let result = move_camera(&motion.target, motion.direction);
                motion.refreshed = Instant::now();
                if let Err(error) = result {
                    let id = motion.id;
                    stop_motion(&mut active);
                    let _ = events.send(Event::CommandFailed(id, error));
                }
            }
        }
    }
}

fn stop_motion(active: &mut Option<Motion>) {
    if let Some(motion) = active.take() {
        let _ = stop(&motion.target);
    }
}

pub fn discover(rtsp_url: &str) -> Result<Target, String> {
    let host = rtsp_url
        .strip_prefix("rtsp://")
        .ok_or("Not an RTSP camera")?
        .split(['/', '?', '#'])
        .next()
        .unwrap_or_default();
    let host = host.split(':').next().unwrap_or_default();
    let ip: Ipv4Addr = host
        .parse()
        .map_err(|_| "PTZ discovery currently requires an IPv4 RTSP camera address".to_owned())?;
    if !(ip.is_private() || ip.is_link_local()) || ip.is_broadcast() || ip.is_unspecified() {
        return Err("PTZ discovery is restricted to private local camera addresses".into());
    }

    let mut last_error = "ONVIF Device service did not respond".to_owned();
    for port in [8899, 80, 8000, 8080] {
        let device = Endpoint {
            ip,
            port,
            path: "/onvif/device_service".into(),
        };
        match probe_device(&device) {
            Ok(target) => return Ok(target),
            Err(error) => {
                if error.contains("authentication required") {
                    return Err(error);
                }
                last_error = error;
            }
        }
    }
    Err(last_error)
}

fn probe_device(device: &Endpoint) -> Result<Target, String> {
    let response = request(
        device,
        "http://www.onvif.org/ver10/device/wsdl/GetCapabilities",
        "<tds:GetCapabilities><tds:Category>All</tds:Category></tds:GetCapabilities>",
    )?;
    let doc = parse_xml(&response)?;
    let services = doc
        .descendants()
        .find(|node| node.is_element() && node.tag_name().name() == "Capabilities")
        .ok_or("ONVIF response has no capabilities")?;

    let ptz = services
        .children()
        .find(|node| node.is_element() && node.tag_name().name() == "PTZ")
        .and_then(|node| child_text(node, "XAddr"))
        .ok_or("This ONVIF device does not advertise PTZ")?;
    let media = services
        .children()
        .find(|node| node.is_element() && node.tag_name().name() == "Media")
        .and_then(|node| child_text(node, "XAddr"))
        .ok_or("This ONVIF device does not advertise a Media service")?;

    let ptz_endpoint = Endpoint::same_camera_url(device.ip, &ptz)?;
    let media_endpoint = Endpoint::same_camera_url(device.ip, &media)?;

    let profiles = request(
        &media_endpoint,
        "http://www.onvif.org/ver10/media/wsdl/GetProfiles",
        "<trt:GetProfiles/>",
    )?;
    let doc = parse_xml(&profiles)?;
    let profile = doc
        .descendants()
        .find(|node| {
            node.is_element()
                && node.tag_name().name() == "Profiles"
                && node.children().any(|child| {
                    child.is_element() && child.tag_name().name() == "PTZConfiguration"
                })
        })
        .ok_or("No ONVIF media profile has PTZ configuration")?;
    let token = profile
        .attribute("token")
        .ok_or("ONVIF PTZ profile has no token")?;
    let token = escape_token(token)?;

    let status =
        format!("<tptz:GetStatus><tptz:ProfileToken>{token}</tptz:ProfileToken></tptz:GetStatus>");
    let status_reply = request(
        &ptz_endpoint,
        "http://www.onvif.org/ver20/ptz/wsdl/GetStatus",
        &status,
    )?;
    let doc = parse_xml(&status_reply)?;
    if !doc
        .descendants()
        .any(|node| node.is_element() && node.tag_name().name() == "GetStatusResponse")
    {
        return Err("Camera did not confirm an ONVIF PTZ status response".into());
    }
    Ok(Target {
        endpoint: ptz_endpoint,
        profile: token,
    })
}

fn move_camera(target: &Target, direction: Direction) -> Result<(), String> {
    let (x, y) = direction.velocity();
    let body = format!(
        "<tptz:ContinuousMove><tptz:ProfileToken>{}</tptz:ProfileToken>\
         <tptz:Velocity><tt:PanTilt x=\"{x}\" y=\"{y}\"/></tptz:Velocity>\
         <tptz:Timeout>{CAMERA_MOVE_TIMEOUT}</tptz:Timeout></tptz:ContinuousMove>",
        target.profile,
    );
    request(
        &target.endpoint,
        "http://www.onvif.org/ver20/ptz/wsdl/ContinuousMove",
        &body,
    )
    .map(|_| ())
}

fn stop(target: &Target) -> Result<(), String> {
    let body = format!(
        "<tptz:Stop><tptz:ProfileToken>{}</tptz:ProfileToken>\
         <tptz:PanTilt>true</tptz:PanTilt><tptz:Zoom>true</tptz:Zoom></tptz:Stop>",
        target.profile,
    );
    request(
        &target.endpoint,
        "http://www.onvif.org/ver20/ptz/wsdl/Stop",
        &body,
    )
    .map(|_| ())
}

impl Endpoint {
    fn same_camera_url(expected_ip: Ipv4Addr, url: &str) -> Result<Self, String> {
        let value = url
            .strip_prefix("http://")
            .ok_or("ONVIF returned a non-HTTP service URL")?;
        let (host, path) = value.split_once('/').ok_or("Invalid ONVIF service URL")?;
        let (ip, port) = if let Some((host, port)) = host.rsplit_once(':') {
            let ip: Ipv4Addr = host.parse().map_err(|_| "Invalid ONVIF service host")?;
            let port: u16 = port.parse().map_err(|_| "Invalid ONVIF service port")?;
            (ip, port)
        } else {
            (host.parse().map_err(|_| "Invalid ONVIF service host")?, 80)
        };
        if ip != expected_ip
            || port == 0
            || path.len() > 1024
            || !path
                .bytes()
                .all(|byte| byte.is_ascii_graphic() && byte != b'#' && byte != b'?')
        {
            return Err("ONVIF service URL must use the camera's configured IP".into());
        }
        Ok(Self {
            ip,
            port,
            path: format!("/{path}"),
        })
    }
}

fn escape_token(value: &str) -> Result<String, String> {
    if value.is_empty()
        || value.len() > 128
        || !value
            .chars()
            .all(|ch| ch.is_ascii_alphanumeric() || matches!(ch, '-' | '_' | '.' | ':' | '@'))
    {
        return Err("Invalid ONVIF profile token".into());
    }
    Ok(value.to_owned())
}

fn child_text(node: roxmltree::Node<'_, '_>, name: &str) -> Option<String> {
    node.children()
        .find(|child| child.is_element() && child.tag_name().name() == name)
        .and_then(|child| child.text())
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_owned)
}

fn parse_xml(body: &str) -> Result<Document<'_>, String> {
    let doc = Document::parse(body).map_err(|_| "Invalid ONVIF XML response")?;
    if doc
        .descendants()
        .any(|node| node.is_element() && node.tag_name().name() == "Fault")
    {
        return Err("Camera returned an ONVIF SOAP fault".into());
    }
    Ok(doc)
}

fn request(endpoint: &Endpoint, action: &str, body: &str) -> Result<String, String> {
    let addr = SocketAddrV4::new(endpoint.ip, endpoint.port).into();
    let mut stream = TcpStream::connect_timeout(&addr, IO_TIMEOUT)
        .map_err(|_| "ONVIF service is not reachable")?;
    stream
        .set_read_timeout(Some(IO_TIMEOUT))
        .map_err(|_| "ONVIF socket error")?;
    stream
        .set_write_timeout(Some(IO_TIMEOUT))
        .map_err(|_| "ONVIF socket error")?;
    let xml = format!(
        "<?xml version=\"1.0\" encoding=\"utf-8\"?>\
         <s:Envelope xmlns:s=\"http://www.w3.org/2003/05/soap-envelope\" \
         xmlns:tds=\"http://www.onvif.org/ver10/device/wsdl\" \
         xmlns:trt=\"http://www.onvif.org/ver10/media/wsdl\" \
         xmlns:tptz=\"http://www.onvif.org/ver20/ptz/wsdl\" \
         xmlns:tt=\"http://www.onvif.org/ver10/schema\"><s:Body>{body}</s:Body></s:Envelope>"
    );
    let headers = format!(
        "POST {} HTTP/1.1\r\nHost: {}:{}\r\n\
         Content-Type: application/soap+xml; charset=utf-8; action=\"{action}\"\r\n\
         Content-Length: {}\r\nConnection: close\r\n\r\n",
        endpoint.path,
        endpoint.ip,
        endpoint.port,
        xml.len(),
    );
    stream
        .write_all(headers.as_bytes())
        .map_err(|_| "Could not send ONVIF request")?;
    stream
        .write_all(xml.as_bytes())
        .map_err(|_| "Could not send ONVIF request")?;

    let mut raw = Vec::with_capacity(4096);
    let mut block = [0; 4096];
    loop {
        match stream.read(&mut block) {
            Ok(0) => break,
            Ok(n) => {
                if raw.len() + n > MAX_REPLY {
                    return Err("ONVIF response exceeds size limit".into());
                }
                raw.extend_from_slice(&block[..n]);
                if let Some(cut) = raw.windows(4).position(|bytes| bytes == b"\r\n\r\n")
                    && let Ok(headers) = std::str::from_utf8(&raw[..cut])
                    && let Some(length) = headers.lines().find_map(|line| {
                        line.to_ascii_lowercase()
                            .strip_prefix("content-length:")
                            .and_then(|value| value.trim().parse::<usize>().ok())
                    })
                {
                    if length > MAX_REPLY {
                        return Err("ONVIF response exceeds size limit".into());
                    }
                    if raw.len() >= cut + 4 + length {
                        break;
                    }
                }
            }
            Err(error) if error.kind() == std::io::ErrorKind::TimedOut => {
                if raw.windows(4).any(|part| part == b"\r\n\r\n") {
                    break;
                }
                return Err("ONVIF device timed out".into());
            }
            Err(_) => return Err("Could not read ONVIF response".into()),
        }
    }

    let cut = raw
        .windows(4)
        .position(|bytes| bytes == b"\r\n\r\n")
        .ok_or("Invalid ONVIF HTTP response")?;
    let headers = std::str::from_utf8(&raw[..cut]).map_err(|_| "Invalid ONVIF HTTP headers")?;
    let code = headers
        .lines()
        .next()
        .and_then(|line| line.split_whitespace().nth(1))
        .and_then(|code| code.parse::<u16>().ok())
        .ok_or("Invalid ONVIF HTTP status")?;
    if code == 401 || code == 403 {
        return Err(
            "ONVIF authentication required; secure login support is not yet available".into(),
        );
    }
    if code != 200 {
        return Err(format!("ONVIF device returned HTTP {code}"));
    }
    if headers.lines().any(|line| {
        line.to_ascii_lowercase().starts_with("transfer-encoding:")
            && line.to_ascii_lowercase().contains("chunked")
    }) {
        return Err("This ONVIF device uses chunked HTTP responses, not yet supported".into());
    }
    let reply = std::str::from_utf8(&raw[cut + 4..]).map_err(|_| "Invalid ONVIF XML encoding")?;
    parse_xml(reply)?;
    Ok(reply.to_owned())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn host_parser_disallows_external_hosts() {
        assert!(discover("rtsp://8.8.8.8/live").is_err());
        assert!(discover("rtsp://example.com/live").is_err());
    }

    #[test]
    fn service_urls_cannot_retarget_other_devices() {
        let ip = Ipv4Addr::new(192, 168, 1, 10);
        assert!(Endpoint::same_camera_url(ip, "http://192.168.1.11:8899/onvif/ptz").is_err());
        assert!(Endpoint::same_camera_url(ip, "https://192.168.1.10/onvif/ptz").is_err());
        assert!(Endpoint::same_camera_url(ip, "http://192.168.1.10:8899/onvif/ptz").is_ok());
    }

    #[test]
    fn invalid_tokens_are_rejected() {
        assert!(escape_token("a&b").is_err());
        assert_eq!(escape_token("profile_1").unwrap(), "profile_1");
    }
}
