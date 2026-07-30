use evdev::{
    AbsInfo, AbsoluteAxisCode, Device, EventType, InputEvent, KeyCode, UinputAbsSetup,
    uinput::VirtualDevice,
};
use std::{
    fs, io,
    path::Path,
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
    thread,
    time::{Duration, Instant},
};

pub struct InputProxy {
    restricted: Arc<AtomicBool>,
    refuel_keyboard: Arc<AtomicBool>,
    refuel_controller: Arc<AtomicBool>,
}

impl InputProxy {
    pub fn start() -> io::Result<Self> {
        let mut keyboard = find_keyboard()?;
        let mut controller = find_device(is_gamepad, "gamepad with right trigger")?;
        let virtual_keyboard = virtual_keyboard(&keyboard)?;
        let virtual_controller = virtual_controller(&controller)?;

        keyboard.grab()?;
        controller.grab()?;
        discard_pending_events(&mut keyboard)?;
        discard_pending_events(&mut controller)?;

        let restricted = Arc::new(AtomicBool::new(false));
        let refuel_keyboard = Arc::new(AtomicBool::new(false));
        let refuel_controller = Arc::new(AtomicBool::new(false));
        let keyboard_restricted = Arc::clone(&restricted);
        let keyboard_refuel = Arc::clone(&refuel_keyboard);
        thread::spawn(move || {
            if let Err(error) = proxy_keyboard(
                keyboard,
                virtual_keyboard,
                keyboard_restricted,
                keyboard_refuel,
            ) {
                eprintln!("keyboard throttle proxy stopped: {error}");
            }
        });
        let controller_restricted = Arc::clone(&restricted);
        let controller_refuel = Arc::clone(&refuel_controller);
        thread::spawn(move || {
            if let Err(error) = proxy_controller(
                controller,
                virtual_controller,
                controller_restricted,
                controller_refuel,
            ) {
                eprintln!("controller throttle proxy stopped: {error}");
            }
        });

        Ok(Self {
            restricted,
            refuel_keyboard,
            refuel_controller,
        })
    }

    pub fn set_restricted(&self, restricted: bool) {
        self.restricted.store(restricted, Ordering::Relaxed);
    }

    pub fn refuel_pressed(&self) -> bool {
        self.refuel_keyboard.load(Ordering::Relaxed)
            || self.refuel_controller.load(Ordering::Relaxed)
    }
}

pub fn limit_digital_throttle(pressed: bool, restricted: bool, elapsed_ms: u64) -> bool {
    pressed && (!restricted || digital_throttle_window(elapsed_ms))
}

pub fn limit_analog_throttle(
    value: i32,
    minimum: i32,
    maximum: i32,
    restricted: bool,
    elapsed_ms: u64,
) -> i32 {
    if !restricted {
        return value;
    }
    let Some(percent) = throttle_percent(elapsed_ms) else {
        return minimum;
    };
    minimum + (value - minimum).clamp(0, maximum - minimum) * percent / 100
}

fn throttle_percent(elapsed_ms: u64) -> Option<i32> {
    let sample = misfire_sample(elapsed_ms, 180, 0x6d2b_79f5);
    if sample % 100 < 18 {
        None
    } else {
        Some(40 + ((sample >> 8) % 29) as i32)
    }
}

fn digital_throttle_window(elapsed_ms: u64) -> bool {
    misfire_sample(elapsed_ms, 220, 0xa511_e9b3) % 100 >= 34
}

fn misfire_sample(elapsed_ms: u64, interval_ms: u64, salt: u64) -> u64 {
    let mut value = (elapsed_ms / interval_ms).wrapping_add(salt);
    value = (value ^ (value >> 30)).wrapping_mul(0xbf58_476d_1ce4_e5b9);
    value = (value ^ (value >> 27)).wrapping_mul(0x94d0_49bb_1331_11eb);
    value ^ (value >> 31)
}

fn find_device(matches: fn(&Device) -> bool, description: &str) -> io::Result<Device> {
    for entry in fs::read_dir("/dev/input")? {
        let path = entry?.path();
        if !path
            .file_name()
            .is_some_and(|name| name.as_encoded_bytes().starts_with(b"event"))
        {
            continue;
        }
        if is_virtual_input_device(&path) {
            continue;
        }
        let Ok(device) = Device::open(&path) else {
            continue;
        };
        let name = device.name().unwrap_or_default();
        if !name.contains("ForzaLife")
            && !name.contains("passthrough")
            && !name.contains("ydotool")
            && matches(&device)
        {
            eprintln!("using {} for {description}", display_device(&path, &device));
            return Ok(device);
        }
    }
    Err(io::Error::new(
        io::ErrorKind::NotFound,
        format!("could not find {description}"),
    ))
}

fn is_virtual_input_device(path: &Path) -> bool {
    let Some(event_name) = path.file_name() else {
        return true;
    };
    let sysfs_path = Path::new("/sys/class/input")
        .join(event_name)
        .join("device");
    fs::canonicalize(sysfs_path)
        .map(|path| is_virtual_sysfs_path(&path))
        .unwrap_or(true)
}

fn is_virtual_sysfs_path(path: &Path) -> bool {
    path.starts_with("/sys/devices/virtual")
}

fn find_keyboard() -> io::Result<Device> {
    for entry in fs::read_dir("/dev/input/by-id")? {
        let path = entry?.path();
        if !path
            .file_name()
            .is_some_and(|name| name.as_encoded_bytes().ends_with(b"-event-kbd"))
        {
            continue;
        }
        let Ok(device) = Device::open(&path) else {
            continue;
        };
        if is_keyboard(&device) {
            eprintln!(
                "using {} for keyboard with W key",
                display_device(&path, &device)
            );
            return Ok(device);
        }
    }
    Err(io::Error::new(
        io::ErrorKind::NotFound,
        "could not find keyboard with W key",
    ))
}

fn is_keyboard(device: &Device) -> bool {
    device.supported_keys().is_some_and(|keys| {
        keys.contains(KeyCode::KEY_W)
            && keys.contains(KeyCode::KEY_A)
            && keys.contains(KeyCode::KEY_SPACE)
    })
}

fn is_gamepad(device: &Device) -> bool {
    device
        .supported_keys()
        .is_some_and(|keys| keys.contains(KeyCode::BTN_SOUTH))
        && device.supported_absolute_axes().is_some_and(|axes| {
            axes.contains(AbsoluteAxisCode::ABS_X) && axes.contains(AbsoluteAxisCode::ABS_RZ)
        })
}

fn display_device(path: &Path, device: &Device) -> String {
    format!(
        "{} ({})",
        device.name().unwrap_or("unnamed input device"),
        path.display()
    )
}

fn virtual_keyboard(device: &Device) -> io::Result<VirtualDevice> {
    let name = device.name().unwrap_or("ForzaLife Keyboard Proxy");
    let keys = device
        .supported_keys()
        .ok_or_else(|| io::Error::other("keyboard has no key capabilities"))?;
    VirtualDevice::builder()?
        .name(name)
        .input_id(device.input_id())
        .with_properties(device.properties())?
        .with_keys(keys)?
        .build()
}

fn virtual_controller(device: &Device) -> io::Result<VirtualDevice> {
    let keys = device
        .supported_keys()
        .ok_or_else(|| io::Error::other("controller has no button capabilities"))?;
    let mut builder = VirtualDevice::builder()?
        .name("ForzaLife Virtual Controller")
        .input_id(device.input_id())
        .with_properties(device.properties())?
        .with_keys(keys)?;
    let axes: Vec<_> = device.get_absinfo()?.collect();
    for (axis, info) in &axes {
        let info = AbsInfo::new(
            initial_axis_value(*axis, *info),
            info.minimum(),
            info.maximum(),
            info.fuzz(),
            info.flat(),
            info.resolution(),
        );
        builder = builder.with_absolute_axis(&UinputAbsSetup::new(*axis, info))?;
    }
    builder.build()
}

fn initial_axis_value(axis: AbsoluteAxisCode, info: AbsInfo) -> i32 {
    if matches!(axis, AbsoluteAxisCode::ABS_Z | AbsoluteAxisCode::ABS_RZ) {
        info.minimum()
    } else {
        0.clamp(info.minimum(), info.maximum())
    }
}

fn discard_pending_events(device: &mut Device) -> io::Result<()> {
    device.set_nonblocking(true)?;
    loop {
        match device.fetch_events() {
            Ok(mut events) => {
                if events.next().is_none() {
                    return Ok(());
                }
            }
            Err(error) if error.kind() == io::ErrorKind::WouldBlock => return Ok(()),
            Err(error) => return Err(error),
        }
    }
}

fn proxy_keyboard(
    mut physical: Device,
    mut virtual_device: VirtualDevice,
    restricted: Arc<AtomicBool>,
    refuel_pressed: Arc<AtomicBool>,
) -> io::Result<()> {
    physical.set_nonblocking(true)?;
    let mut pressed = false;
    let mut emitted = false;
    let started = Instant::now();

    loop {
        let mut output = Vec::new();
        match physical.fetch_events() {
            Ok(events) => {
                for event in events {
                    if event.event_type() == EventType::KEY {
                        if event.code() == KeyCode::KEY_ENTER.code() {
                            refuel_pressed.store(event.value() != 0, Ordering::Relaxed);
                        }
                        if event.code() == KeyCode::KEY_W.code() {
                            pressed = event.value() != 0;
                            continue;
                        }
                        output.push(event);
                    } else if event.event_type() != EventType::SYNCHRONIZATION {
                        output.push(event);
                    }
                }
            }
            Err(error) if error.kind() == io::ErrorKind::WouldBlock => {}
            Err(error) => return Err(error),
        }

        let desired = limit_digital_throttle(
            pressed,
            restricted.load(Ordering::Relaxed),
            started.elapsed().as_millis() as u64,
        );
        if desired != emitted {
            output.push(InputEvent::new(
                EventType::KEY.0,
                KeyCode::KEY_W.code(),
                i32::from(desired),
            ));
            emitted = desired;
        }
        if !output.is_empty() {
            virtual_device.emit(&output)?;
        }
        thread::sleep(Duration::from_millis(8));
    }
}

fn proxy_controller(
    mut physical: Device,
    mut virtual_device: VirtualDevice,
    restricted: Arc<AtomicBool>,
    refuel_pressed: Arc<AtomicBool>,
) -> io::Result<()> {
    let trigger = physical
        .get_absinfo()?
        .find(|(axis, _)| *axis == AbsoluteAxisCode::ABS_RZ)
        .map(|(_, info)| info)
        .ok_or_else(|| io::Error::other("controller has no right-trigger axis"))?;
    physical.set_nonblocking(true)?;
    let mut trigger_value = trigger.minimum();
    let mut emitted_trigger = trigger.minimum();
    let started = Instant::now();

    loop {
        let mut output = Vec::new();
        match physical.fetch_events() {
            Ok(events) => {
                for event in events {
                    if event.event_type() == EventType::ABSOLUTE {
                        if event.code() == AbsoluteAxisCode::ABS_RZ.0 {
                            trigger_value = event.value();
                        } else {
                            output.push(event);
                        }
                    } else if event.event_type() != EventType::SYNCHRONIZATION {
                        if event.event_type() == EventType::KEY
                            && event.code() == KeyCode::BTN_EAST.code()
                        {
                            refuel_pressed.store(event.value() != 0, Ordering::Relaxed);
                        }
                        output.push(event);
                    }
                }
            }
            Err(error) if error.kind() == io::ErrorKind::WouldBlock => {}
            Err(error) => return Err(error),
        }

        let desired_trigger = limit_analog_throttle(
            trigger_value,
            trigger.minimum(),
            trigger.maximum(),
            restricted.load(Ordering::Relaxed),
            started.elapsed().as_millis() as u64,
        );
        if desired_trigger != emitted_trigger {
            output.push(InputEvent::new(
                EventType::ABSOLUTE.0,
                AbsoluteAxisCode::ABS_RZ.0,
                desired_trigger,
            ));
            emitted_trigger = desired_trigger;
        }
        if !output.is_empty() {
            virtual_device.emit(&output)?;
        }
        thread::sleep(Duration::from_millis(8));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn virtual_controller_starts_neutral_instead_of_replaying_cached_input() {
        let stick = AbsInfo::new(24_679, -32_768, 32_767, 0, 128, 0);
        let trigger = AbsInfo::new(379, 0, 1_023, 0, 0, 0);

        assert_eq!(initial_axis_value(AbsoluteAxisCode::ABS_X, stick), 0);
        assert_eq!(
            initial_axis_value(AbsoluteAxisCode::ABS_RZ, trigger),
            trigger.minimum()
        );
    }

    #[test]
    fn virtual_input_devices_are_rejected_as_proxy_sources() {
        assert!(is_virtual_sysfs_path(Path::new(
            "/sys/devices/virtual/input/input85"
        )));
        assert!(!is_virtual_sysfs_path(Path::new(
            "/sys/devices/pci0000:00/0000:00:01.3/input/input69"
        )));
    }
}
