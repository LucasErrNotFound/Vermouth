#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct CameraId(pub u64);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ConnectionState {
    PreviewUnavailable,
    Connecting,
    Offline,
    Unauthorized,
    Unsupported,
}

impl ConnectionState {
    pub fn label(self) -> &'static str {
        match self {
            Self::PreviewUnavailable => "Demo · Preview unavailable",
            Self::Connecting => "Demo · Connecting",
            Self::Offline => "Demo · Offline",
            Self::Unauthorized => "Demo · Authentication needed",
            Self::Unsupported => "Demo · Unsupported",
        }
    }

    pub fn description(self) -> &'static str {
        match self {
            Self::PreviewUnavailable => "No video is connected in the frontend demonstrator.",
            Self::Connecting => "Simulated connection state. No network request is being made.",
            Self::Offline => "The simulated device is unavailable.",
            Self::Unauthorized => {
                "Simulated authentication failure. No credentials are required here."
            }
            Self::Unsupported => "This simulated configuration has an unsupported feature.",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CameraTransport {
    Rtsp,
    OnvifRtsp,
}

impl CameraTransport {
    pub fn label(self) -> &'static str {
        match self {
            Self::Rtsp => "RTSP",
            Self::OnvifRtsp => "ONVIF + RTSP",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Capability {
    Unknown,
    Advertised,
    Verified,
    Unsupported,
}

impl Capability {
    pub fn label(self) -> &'static str {
        match self {
            Self::Unknown => "Unknown",
            Self::Advertised => "Advertised (demo)",
            Self::Verified => "Verified (demo fixture only)",
            Self::Unsupported => "Unsupported",
        }
    }
}

#[derive(Debug, Clone)]
pub struct Camera {
    pub id: CameraId,
    pub name: String,
    pub rtsp_address: String,
    pub transport: CameraTransport,
    pub connection: ConnectionState,
    pub ptz: Capability,
}

pub struct CameraCatalog {
    records: Vec<Camera>,
    next_id: u64,
}

impl CameraCatalog {
    pub fn demo() -> Self {
        let mut catalog = Self {
            records: Vec::new(),
            next_id: 1,
        };
        for (name, path, transport, connection, ptz) in [
            (
                "Front Entrance",
                "front",
                CameraTransport::OnvifRtsp,
                ConnectionState::PreviewUnavailable,
                Capability::Advertised,
            ),
            (
                "Driveway",
                "driveway",
                CameraTransport::Rtsp,
                ConnectionState::Offline,
                Capability::Unsupported,
            ),
            (
                "Hallway",
                "hallway",
                CameraTransport::OnvifRtsp,
                ConnectionState::Connecting,
                Capability::Verified,
            ),
            (
                "Workshop",
                "workshop",
                CameraTransport::Rtsp,
                ConnectionState::Unauthorized,
                Capability::Unsupported,
            ),
            (
                "Back Garden",
                "garden",
                CameraTransport::Rtsp,
                ConnectionState::Unsupported,
                Capability::Unsupported,
            ),
        ] {
            let id = CameraId(catalog.next_id);
            catalog.next_id += 1;
            catalog.records.push(Camera {
                id,
                name: name.to_owned(),
                rtsp_address: format!("rtsp://demo.invalid/{path}"),
                transport,
                connection,
                ptz,
            });
        }
        catalog
    }

    pub fn all(&self) -> &[Camera] {
        &self.records
    }
    pub fn get(&self, id: CameraId) -> Option<&Camera> {
        self.records.iter().find(|c| c.id == id)
    }

    pub fn add(&mut self, name: &str, address: &str, onvif: bool) -> Result<CameraId, String> {
        self.validate(None, name, address)?;
        let id = CameraId(self.next_id);
        self.next_id += 1;
        self.records.push(Camera {
            id,
            name: name.trim().to_owned(),
            rtsp_address: address.trim().to_owned(),
            transport: if onvif {
                CameraTransport::OnvifRtsp
            } else {
                CameraTransport::Rtsp
            },
            connection: ConnectionState::PreviewUnavailable,
            ptz: Capability::Unknown,
        });
        Ok(id)
    }

    pub fn update(
        &mut self,
        id: CameraId,
        name: &str,
        address: &str,
        onvif: bool,
    ) -> Result<(), String> {
        self.validate(Some(id), name, address)?;
        let camera = self
            .records
            .iter_mut()
            .find(|c| c.id == id)
            .ok_or("Camera no longer exists")?;
        camera.name = name.trim().to_owned();
        camera.rtsp_address = address.trim().to_owned();
        camera.transport = if onvif {
            CameraTransport::OnvifRtsp
        } else {
            CameraTransport::Rtsp
        };
        Ok(())
    }

    pub fn remove(&mut self, id: CameraId) -> bool {
        let before = self.records.len();
        self.records.retain(|c| c.id != id);
        before != self.records.len()
    }

    fn validate(&self, editing: Option<CameraId>, name: &str, address: &str) -> Result<(), String> {
        let name = name.trim();
        let address = address.trim();
        if name.is_empty() {
            return Err("Enter a camera name.".into());
        }
        if name.chars().count() > 80 {
            return Err("Camera name must have at most 80 characters.".into());
        }
        if address.len() > 2048 {
            return Err("RTSP address is too long.".into());
        }
        if address.contains('?') || address.contains('#') {
            return Err("Query strings and fragments are not accepted in this demo because they may contain secrets.".into());
        }
        if !(address.starts_with("rtsp://") || address.starts_with("rtsps://")) {
            return Err("Address must begin with rtsp:// or rtsps://.".into());
        }
        let authority = address
            .split_once("://")
            .map(|(_, rest)| rest.split('/').next().unwrap_or(""))
            .unwrap_or("");
        if authority.is_empty()
            || authority.contains('@')
            || authority.chars().any(char::is_whitespace)
        {
            return Err("Enter a host without embedded credentials or spaces.".into());
        }
        if self
            .records
            .iter()
            .any(|c| Some(c.id) != editing && c.rtsp_address.eq_ignore_ascii_case(address))
        {
            return Err("A camera with this RTSP address already exists.".into());
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fixture_is_synthetic_and_covers_failure_modes() {
        let cameras = CameraCatalog::demo();
        assert_eq!(cameras.all().len(), 5);
        assert!(
            cameras
                .all()
                .iter()
                .all(|c| c.rtsp_address.contains("demo.invalid"))
        );
        assert!(
            cameras
                .all()
                .iter()
                .all(|c| c.connection != ConnectionState::PreviewUnavailable
                    || c.connection.label().starts_with("Demo"))
        );
    }

    #[test]
    fn add_edit_delete_and_duplicate_validation() {
        let mut catalog = CameraCatalog::demo();
        let id = catalog
            .add("Office", "rtsp://demo.invalid/office", false)
            .unwrap();
        assert!(
            catalog
                .add("Duplicate", "rtsp://demo.invalid/office", false)
                .is_err()
        );
        assert!(
            catalog
                .add("Secret", "rtsp://admin:password@demo.invalid/stream", false)
                .is_err()
        );
        assert!(
            catalog
                .add("Secret", "rtsp://demo.invalid/stream?token=secret", false)
                .is_err()
        );
        assert!(
            catalog
                .update(id, "Office 2", "rtsp://demo.invalid/new", true)
                .is_ok()
        );
        assert_eq!(
            catalog.get(id).unwrap().transport,
            CameraTransport::OnvifRtsp
        );
        assert!(catalog.remove(id));
        assert!(catalog.get(id).is_none());
    }
}
