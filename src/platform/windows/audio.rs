//! Audio: mute and volume of the default output with Core Audio.
//!
//! The endpoint is re-enumerated every 2 s: that detects device changes
//! (headphones, HDMI) without implementing `IMMNotificationClient`. Mute
//! changes show up within that interval at most.

use std::time::{Duration, Instant};

use windows::Win32::Media::Audio::Endpoints::IAudioEndpointVolume;
use windows::Win32::Media::Audio::{IMMDeviceEnumerator, MMDeviceEnumerator, eConsole, eRender};
use windows::Win32::System::Com::{
    CLSCTX_ALL, COINIT_MULTITHREADED, CoCreateInstance, CoInitializeEx, CoUninitialize,
};

use crate::event::{Audio, Event};

const POLL: Duration = Duration::from_secs(2);
/// Re-enumerating the endpoint detects device changes; it is not needed on every poll.
const REFRESH: Duration = Duration::from_secs(10);

pub(crate) fn listen(emit: impl Fn(Event) -> bool + Send + 'static) {
    std::thread::spawn(move || {
        // SAFETY: one COM initialization per thread; `ComGuard` balances the count.
        let initialized = unsafe { CoInitializeEx(None, COINIT_MULTITHREADED) };
        if initialized.is_err() {
            return;
        }
        let _guard = ComGuard;

        let mut volume: Option<IAudioEndpointVolume> = None;
        let mut refreshed = Instant::now() - REFRESH;
        let mut last = None;

        loop {
            if refreshed.elapsed() >= REFRESH {
                volume = open();
                refreshed = Instant::now();
            }

            if let Some(endpoint) = &volume {
                match read(endpoint) {
                    Some(audio) if last != Some(audio) => {
                        last = Some(audio);
                        if !emit(Event::Audio(audio)) {
                            return;
                        }
                    }
                    Some(_) => {}
                    None => volume = None,
                }
            }

            std::thread::sleep(POLL);
        }
    });
}

struct ComGuard;

impl Drop for ComGuard {
    fn drop(&mut self) {
        // SAFETY: balances the `CoInitializeEx` of this same thread.
        unsafe {
            CoUninitialize();
        }
    }
}

fn open() -> Option<IAudioEndpointVolume> {
    // SAFETY: COM is initialized on this thread; the interfaces are only used
    // here and released when the function returns.
    unsafe {
        let enumerator: IMMDeviceEnumerator =
            CoCreateInstance(&MMDeviceEnumerator, None, CLSCTX_ALL).ok()?;
        let device = enumerator.GetDefaultAudioEndpoint(eRender, eConsole).ok()?;
        let volume: IAudioEndpointVolume = device.Activate(CLSCTX_ALL, None).ok()?;

        Some(volume)
    }
}

fn read(volume: &IAudioEndpointVolume) -> Option<Audio> {
    // SAFETY: the interface belongs to this MTA thread and stays alive during the call.
    unsafe {
        Some(Audio {
            muted: volume.GetMute().ok()?.as_bool(),
            volume: volume_percent(volume.GetMasterVolumeLevelScalar().ok()?),
        })
    }
}

fn volume_percent(scalar: f32) -> u16 {
    (scalar.clamp(0.0, 1.0) * 100.0).round() as u16
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn converts_scalar_volume() {
        assert_eq!(volume_percent(0.0), 0);
        assert_eq!(volume_percent(0.5), 50);
        assert_eq!(volume_percent(1.0), 100);
        assert_eq!(volume_percent(1.5), 100);
    }
}
