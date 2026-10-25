use std::{
    io::{self, Read},
    process::{Child, Command, Stdio},
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, Ordering},
    },
    thread,
};

use gpui_kit::RenderImage;
use image::{Frame, RgbaImage};
use smallvec::smallvec;

const FRAME_WIDTH: u32 = 960;
const FRAME_HEIGHT: u32 = 540;
const FRAME_BYTES: usize = FRAME_WIDTH as usize * FRAME_HEIGHT as usize * 4;

pub enum StreamEvent {
    Frame(Arc<RenderImage>),
    Failed(&'static str),
}

pub struct StreamSession {
    cancelled: Arc<AtomicBool>,
    process: Arc<Mutex<Option<Child>>>,
    mailbox: Arc<Mutex<Option<StreamEvent>>>,
}

impl StreamSession {
    pub fn start(url: String) -> Self {
        let cancelled = Arc::new(AtomicBool::new(false));
        let process = Arc::new(Mutex::new(None));
        let mailbox = Arc::new(Mutex::new(None));

        let stopped = cancelled.clone();
        let current = process.clone();
        let events = mailbox.clone();

        thread::spawn(move || {
            let mut command = Command::new("ffmpeg");

            command.args([
                "-hide_banner",
                "-loglevel",
                "error",
                "-nostdin",
                "-fflags",
                "nobuffer",
                "-flags",
                "low_delay",
                "-rtsp_transport",
                "tcp",
                "-timeout",
                "15000000",
                "-max_delay",
                "500000",
                "-analyzeduration",
                "1000000",
                "-probesize",
                "1000000",
                "-i",
            ]);

            command.arg(&url);

            command.args([
                "-map",
                "0:v:0",
                "-an",
                "-sn",
                "-dn",
                "-vf",
                "fps=15,scale=w=960:h=540:force_original_aspect_ratio=decrease,pad=960:540:(ow-iw)/2:(oh-ih)/2",
                "-pix_fmt",
                "bgra",
                "-fps_mode",
                "passthrough",
                "-f",
                "rawvideo",
                "pipe:1",
            ]);

            command
                .stdin(Stdio::null())
                .stdout(Stdio::piped())
                .stderr(Stdio::null());

            #[cfg(windows)]
            {
                use std::os::windows::process::CommandExt;
                command.creation_flags(0x08000000);
            }

            let mut child = match command.spawn() {
                Ok(child) => child,
                Err(_) => {
                    if !stopped.load(Ordering::Relaxed) {
                        publish(
                            &events,
                            StreamEvent::Failed(
                                "FFmpeg could not be started. Check its installation.",
                            ),
                        );
                    }
                    return;
                }
            };

            let Some(mut stdout) = child.stdout.take() else {
                let _ = child.kill();
                let _ = child.wait();

                if !stopped.load(Ordering::Relaxed) {
                    publish(
                        &events,
                        StreamEvent::Failed("Decoder output is unavailable."),
                    );
                }
                return;
            };

            {
                let mut slot = current.lock().unwrap_or_else(|error| error.into_inner());

                if stopped.load(Ordering::Relaxed) {
                    let _ = child.kill();
                    let _ = child.wait();
                    return;
                }

                *slot = Some(child);
            }

            let outcome = read_frames(&mut stdout, &stopped, |frame| {
                publish(&events, StreamEvent::Frame(frame));
            });

            {
                let mut slot = current.lock().unwrap_or_else(|error| error.into_inner());

                if let Some(mut child) = slot.take() {
                    let _ = child.kill();
                    let _ = child.wait();
                }
            }

            if !stopped.load(Ordering::Relaxed) {
                let reason = match outcome {
                    Ok(()) => "Video stream ended unexpectedly.",
                    Err(_) => "Video transport stopped or the decoder failed.",
                };

                publish(&events, StreamEvent::Failed(reason));
            }
        });

        Self {
            cancelled,
            process,
            mailbox,
        }
    }

    pub fn take_event(&self) -> Option<StreamEvent> {
        self.mailbox
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .take()
    }

    pub fn stop(&self) {
        self.cancelled.store(true, Ordering::Relaxed);

        let mut slot = self
            .process
            .lock()
            .unwrap_or_else(|error| error.into_inner());

        if let Some(child) = slot.as_mut() {
            let _ = child.kill();
        }
    }
}

impl Drop for StreamSession {
    fn drop(&mut self) {
        self.stop();
    }
}

fn publish(mailbox: &Mutex<Option<StreamEvent>>, event: StreamEvent) {
    let mut slot = mailbox.lock().unwrap_or_else(|error| error.into_inner());

    *slot = Some(event);
}

fn read_frames(
    reader: &mut impl Read,
    cancelled: &AtomicBool,
    mut deliver: impl FnMut(Arc<RenderImage>),
) -> io::Result<()> {
    loop {
        if cancelled.load(Ordering::Relaxed) {
            return Ok(());
        }

        let mut pixels = vec![0u8; FRAME_BYTES];

        reader.read_exact(&mut pixels)?;

        if cancelled.load(Ordering::Relaxed) {
            return Ok(());
        }

        let Some(buffer) = RgbaImage::from_raw(FRAME_WIDTH, FRAME_HEIGHT, pixels) else {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "Invalid video frame dimensions",
            ));
        };

        let frame = Frame::new(buffer);
        let image = Arc::new(RenderImage::new(smallvec![frame]));

        deliver(image);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn raw_video_frame_has_expected_dimensions() {
        let pixels = vec![0u8; FRAME_BYTES];

        let buffer = RgbaImage::from_raw(FRAME_WIDTH, FRAME_HEIGHT, pixels).unwrap();

        assert_eq!(buffer.width(), 960);
        assert_eq!(buffer.height(), 540);
        assert_eq!(buffer.as_raw().len(), FRAME_BYTES,);
    }

    #[test]
    fn latest_event_replaces_previous_event() {
        let mailbox = Mutex::new(None);

        publish(&mailbox, StreamEvent::Failed("First error"));

        publish(&mailbox, StreamEvent::Failed("Latest error"));

        let event = mailbox.lock().unwrap().take();

        assert!(matches!(event, Some(StreamEvent::Failed("Latest error"))));
    }
}
