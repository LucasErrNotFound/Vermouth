pub mod ptz;
mod stream;

use std::{
    collections::HashMap,
    sync::Arc,
    time::{Duration, Instant},
};

use gpui_kit::RenderImage;
use stream::{StreamEvent, StreamSession};

use crate::persistence::cameras::{self as camera_storage, SavedCamera};

const INITIAL_RETRY_SECONDS: u64 = 2;
const MAX_RETRY_SECONDS: u64 = 30;
const CONNECT_TIMEOUT_SECONDS: u64 = 20;
const STALE_AFTER_SECONDS: u64 = 6;
const STALE_TIMEOUT_SECONDS: u64 = 18;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct CameraId(pub u64);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ConnectionState {
    Offline,
    Connecting,
    Live,
    Stale,
    Error,
}

impl ConnectionState {
    pub fn label(self) -> &'static str {
        match self {
            Self::Offline => "Disconnected",
            Self::Connecting => "Connecting...",
            Self::Live => "Live",
            Self::Stale => "Stream stale",
            Self::Error => "Connection failed",
        }
    }

    pub fn description(self) -> &'static str {
        match self {
            Self::Offline => "The stream is disconnected.",
            Self::Connecting => "Waiting for the first decoded video frame.",
            Self::Live => "Decoded video frames are arriving.",
            Self::Stale => "No recent video frames have arrived.",
            Self::Error => "The stream failed. Vermouth will retry automatically.",
        }
    }
}

#[derive(Debug, Clone)]
pub struct Camera {
    pub id: CameraId,
    pub name: String,
    pub rtsp_address: String,
    pub connection: ConnectionState,
    pub detail: Option<&'static str>,
    pub latest_frame: Option<Arc<RenderImage>>,
    pub frame_revision: u64,
    pub last_frame_at: Option<Instant>,
    started_at: Option<Instant>,
    stable_since: Option<Instant>,
    wants_connection: bool,
    retry_delay_seconds: u64,
    next_retry_at: Option<Instant>,
}

pub struct CameraCatalog {
    records: Vec<Camera>,
    next_id: u64,
    sessions: HashMap<CameraId, StreamSession>,
    storage_error: Option<String>,
    load_error: Option<String>,
}

impl CameraCatalog {
    pub fn new() -> Self {
        match camera_storage::load() {
            Ok(saved) => {
                let next_id = saved
                    .iter()
                    .map(|camera| camera.id)
                    .max()
                    .unwrap_or(0)
                    .saturating_add(1);

                let records = saved
                    .into_iter()
                    .map(|saved| {
                        Self::offline_camera(CameraId(saved.id), saved.name, saved.rtsp_address)
                    })
                    .collect();

                Self {
                    records,
                    next_id,
                    sessions: HashMap::new(),
                    storage_error: None,
                    load_error: None,
                }
            }
            Err(message) => Self {
                records: Vec::new(),
                next_id: 1,
                sessions: HashMap::new(),
                storage_error: Some(message.clone()),
                load_error: Some(message),
            },
        }
    }

    fn offline_camera(id: CameraId, name: String, rtsp_address: String) -> Camera {
        Camera {
            id,
            name,
            rtsp_address,
            connection: ConnectionState::Offline,
            detail: None,
            latest_frame: None,
            frame_revision: 0,
            last_frame_at: None,
            started_at: None,
            stable_since: None,
            wants_connection: false,
            retry_delay_seconds: INITIAL_RETRY_SECONDS,
            next_retry_at: None,
        }
    }

    /// A failed load blocks writes to avoid overwriting a damaged or unreadable file.
    pub fn storage_error(&self) -> Option<&str> {
        self.storage_error.as_deref()
    }

    fn save_records(&self, records: &[Camera]) -> Result<(), String> {
        if let Some(error) = &self.load_error {
            return Err(format!(
                "Saved cameras could not be loaded ({error}). Fix or back up the file before editing cameras."
            ));
        }

        let snapshot = records
            .iter()
            .map(|camera| SavedCamera {
                id: camera.id.0,
                name: camera.name.clone(),
                rtsp_address: camera.rtsp_address.clone(),
            })
            .collect::<Vec<_>>();

        camera_storage::save(&snapshot)
    }

    pub fn all(&self) -> &[Camera] {
        &self.records
    }

    pub fn get(&self, id: CameraId) -> Option<&Camera> {
        self.records.iter().find(|camera| camera.id == id)
    }

    pub fn add(&mut self, name: &str, address: &str) -> Result<CameraId, String> {
        self.validate(None, name, address)?;

        let id = CameraId(self.next_id);
        let following_id = self
            .next_id
            .checked_add(1)
            .ok_or("No camera IDs remain available")?;

        let mut proposed = self.records.clone();
        proposed.push(Self::offline_camera(
            id,
            name.trim().to_owned(),
            address.trim().to_owned(),
        ));

        if let Err(error) = self.save_records(&proposed) {
            self.storage_error = Some(error.clone());
            return Err(error);
        }

        self.storage_error = None;
        self.records = proposed;
        self.next_id = following_id;
        self.connect(id)?;
        Ok(id)
    }

    pub fn update(&mut self, id: CameraId, name: &str, address: &str) -> Result<(), String> {
        if self.get(id).is_none() {
            return Err("Camera no longer exists".into());
        }

        self.validate(Some(id), name, address)?;

        let mut proposed = self.records.clone();
        let camera = proposed
            .iter_mut()
            .find(|camera| camera.id == id)
            .expect("camera exists");

        camera.name = name.trim().to_owned();
        camera.rtsp_address = address.trim().to_owned();

        if let Err(error) = self.save_records(&proposed) {
            self.storage_error = Some(error.clone());
            return Err(error);
        }

        self.storage_error = None;
        self.disconnect(id);

        let camera = self
            .records
            .iter_mut()
            .find(|camera| camera.id == id)
            .expect("camera exists");
        camera.name = name.trim().to_owned();
        camera.rtsp_address = address.trim().to_owned();

        self.connect(id)
    }

    pub fn remove(&mut self, id: CameraId) -> bool {
        if self.get(id).is_none() {
            return false;
        }

        let proposed = self
            .records
            .iter()
            .filter(|camera| camera.id != id)
            .cloned()
            .collect::<Vec<_>>();

        if let Err(error) = self.save_records(&proposed) {
            self.storage_error = Some(error);
            return false;
        }

        self.storage_error = None;
        self.disconnect(id);
        self.records.retain(|camera| camera.id != id);
        true
    }

    pub fn connect(&mut self, id: CameraId) -> Result<(), String> {
        self.start_connection(id, true)
    }

    fn start_connection(&mut self, id: CameraId, reset_backoff: bool) -> Result<(), String> {
        self.sessions.remove(&id);

        let camera = self
            .records
            .iter_mut()
            .find(|camera| camera.id == id)
            .ok_or("Camera no longer exists")?;

        if reset_backoff {
            camera.retry_delay_seconds = INITIAL_RETRY_SECONDS;
        }

        camera.wants_connection = true;
        camera.connection = ConnectionState::Connecting;
        camera.detail = None;
        camera.started_at = Some(Instant::now());
        camera.stable_since = None;
        camera.last_frame_at = None;
        camera.next_retry_at = None;

        let session = StreamSession::start(camera.rtsp_address.clone());

        self.sessions.insert(id, session);
        Ok(())
    }

    pub fn disconnect(&mut self, id: CameraId) {
        self.sessions.remove(&id);

        if let Some(camera) = self.records.iter_mut().find(|camera| camera.id == id) {
            camera.wants_connection = false;
            camera.connection = ConnectionState::Offline;
            camera.detail = None;
            camera.latest_frame = None;
            camera.last_frame_at = None;
            camera.started_at = None;
            camera.stable_since = None;
            camera.next_retry_at = None;
            camera.retry_delay_seconds = INITIAL_RETRY_SECONDS;
        }
    }

    fn mark_failure(camera: &mut Camera, reason: &'static str, now: Instant) {
        camera.connection = ConnectionState::Error;
        camera.detail = Some(reason);
        camera.started_at = None;
        camera.stable_since = None;

        if camera.wants_connection {
            camera.next_retry_at = Some(now + Duration::from_secs(camera.retry_delay_seconds));

            camera.retry_delay_seconds = camera
                .retry_delay_seconds
                .saturating_mul(2)
                .min(MAX_RETRY_SECONDS);
        } else {
            camera.next_retry_at = None;
        }
    }

    pub fn poll(&mut self) -> bool {
        let now = Instant::now();
        let mut changed = false;

        let events = self
            .sessions
            .iter()
            .filter_map(|(&id, session)| session.take_event().map(|event| (id, event)))
            .collect::<Vec<_>>();

        for (id, event) in events {
            match event {
                StreamEvent::Frame(frame) => {
                    if let Some(camera) = self
                        .records
                        .iter_mut()
                        .find(|camera| camera.id == id && camera.wants_connection)
                    {
                        if camera.stable_since.is_none() {
                            camera.stable_since = Some(now);
                        }

                        camera.latest_frame = Some(frame);
                        camera.frame_revision = camera.frame_revision.wrapping_add(1);
                        camera.last_frame_at = Some(now);
                        camera.connection = ConnectionState::Live;
                        camera.detail = None;

                        changed = true;
                    }
                }

                StreamEvent::Failed(reason) => {
                    if let Some(camera) = self.records.iter_mut().find(|camera| camera.id == id) {
                        Self::mark_failure(camera, reason, now);
                        changed = true;
                    }

                    self.sessions.remove(&id);
                }
            }
        }

        for camera in &mut self.records {
            if camera.connection == ConnectionState::Live
                && camera
                    .stable_since
                    .is_some_and(|since| now.duration_since(since) >= Duration::from_secs(30))
            {
                camera.retry_delay_seconds = INITIAL_RETRY_SECONDS;
            }

            if camera.connection == ConnectionState::Live
                && camera.last_frame_at.is_some_and(|last| {
                    now.duration_since(last) >= Duration::from_secs(STALE_AFTER_SECONDS)
                })
            {
                camera.connection = ConnectionState::Stale;
                changed = true;
            }
        }

        let expired = self
            .records
            .iter()
            .filter_map(|camera| {
                if !camera.wants_connection {
                    return None;
                }

                let timed_out = match camera.connection {
                    ConnectionState::Connecting => camera.started_at.is_some_and(|started| {
                        now.duration_since(started) >= Duration::from_secs(CONNECT_TIMEOUT_SECONDS)
                    }),
                    ConnectionState::Stale => camera.last_frame_at.is_some_and(|last| {
                        now.duration_since(last) >= Duration::from_secs(STALE_TIMEOUT_SECONDS)
                    }),
                    _ => false,
                };

                if timed_out { Some(camera.id) } else { None }
            })
            .collect::<Vec<_>>();

        for id in expired {
            self.sessions.remove(&id);

            if let Some(camera) = self.records.iter_mut().find(|camera| camera.id == id) {
                Self::mark_failure(camera, "The stream stopped delivering video frames.", now);

                changed = true;
            }
        }

        let retry_ids = self
            .records
            .iter()
            .filter(|camera| {
                camera.wants_connection
                    && camera.connection == ConnectionState::Error
                    && camera.next_retry_at.is_some_and(|time| now >= time)
            })
            .map(|camera| camera.id)
            .collect::<Vec<_>>();

        for id in retry_ids {
            if self.start_connection(id, false).is_ok() {
                changed = true;
            }
        }

        changed
    }

    fn validate(&self, editing: Option<CameraId>, name: &str, address: &str) -> Result<(), String> {
        let name = name.trim();
        let address = address.trim();

        if name.is_empty() || name.chars().count() > 80 {
            return Err("Camera name must contain 1–80 characters.".into());
        }

        if address.len() > 2048 || address.contains(char::is_whitespace) {
            return Err("Enter a valid RTSP address with no spaces.".into());
        }

        let Some(rest) = address.strip_prefix("rtsp://") else {
            return Err("Enter an rtsp:// URL.".into());
        };

        let authority = rest.split(['/', '?', '#']).next().unwrap_or("");

        if authority.is_empty()
            || authority.contains('@')
            || address.contains('?')
            || address.contains('#')
        {
            return Err(
                "Embedded credentials and query tokens are not supported. Use a credential-free RTSP URL."
                    .into(),
            );
        }

        if self.records.iter().any(|camera| {
            Some(camera.id) != editing && camera.rtsp_address.eq_ignore_ascii_case(address)
        }) {
            return Err("This RTSP address is already configured.".into());
        }

        Ok(())
    }
}

impl Drop for CameraCatalog {
    fn drop(&mut self) {
        self.sessions.clear();
    }
}

impl Default for CameraCatalog {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn restored_camera_starts_disconnected() {
        let camera = CameraCatalog::offline_camera(
            CameraId(42),
            "Driveway".into(),
            "rtsp://192.168.1.20/live/ch00_1".into(),
        );

        assert_eq!(camera.connection, ConnectionState::Offline);
        assert!(!camera.wants_connection);
        assert!(camera.latest_frame.is_none());
    }

    #[test]
    fn rejects_unsafe_rtsp_addresses() {
        let mut catalog = CameraCatalog::new();

        assert!(catalog.add("", "rtsp://192.168.1.10/live").is_err());

        assert!(
            catalog
                .add("Front", "rtsp://user:pass@192.168.1.10/live",)
                .is_err()
        );

        assert!(
            catalog
                .add("Front", "rtsp://192.168.1.10/live?token=secret",)
                .is_err()
        );
    }

    #[test]
    fn retry_delay_is_bounded() {
        let now = Instant::now();

        let mut camera = Camera {
            id: CameraId(1),
            name: "Test".into(),
            rtsp_address: "rtsp://example.invalid/live".into(),
            connection: ConnectionState::Connecting,
            detail: None,
            latest_frame: None,
            frame_revision: 0,
            last_frame_at: None,
            started_at: None,
            stable_since: None,
            wants_connection: true,
            retry_delay_seconds: INITIAL_RETRY_SECONDS,
            next_retry_at: None,
        };

        for _ in 0..20 {
            CameraCatalog::mark_failure(&mut camera, "Test failure", now);
        }

        assert_eq!(camera.retry_delay_seconds, MAX_RETRY_SECONDS);
        assert!(camera.next_retry_at.is_some());
    }
}
