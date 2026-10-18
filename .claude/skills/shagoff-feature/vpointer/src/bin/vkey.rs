//! `vkey [ctrl] [alt] [shift] [ru] KEY`: press KEY (a-z, or an evdev code) through
//! zwp_virtual_keyboard_v1 with a plain us,ru keymap, so the app sees the real physical key
//! (wtype uploads a keymap of its own: Ctrl+letters bound to physical keys never arrive). `ru`:
//! the second layout group. For headless sway only (tests).
use std::io::Write;
use std::os::fd::AsFd;
use wayland_client::protocol::wl_keyboard::{KeyState, KeymapFormat};
use wayland_client::protocol::{wl_registry, wl_seat};
use wayland_client::{Connection, Dispatch, QueueHandle, delegate_noop};
use wayland_protocols_misc::zwp_virtual_keyboard_v1::client::{
    zwp_virtual_keyboard_manager_v1::ZwpVirtualKeyboardManagerV1,
    zwp_virtual_keyboard_v1::ZwpVirtualKeyboardV1,
};

const KEYMAP: &str = "xkb_keymap {
  xkb_keycodes { include \"evdev+aliases(qwerty)\" };
  xkb_types { include \"complete\" };
  xkb_compat { include \"complete\" };
  xkb_symbols { include \"pc+us+ru:2+inet(evdev)\" };
};\0";

#[derive(Default)]
struct State {
    seat: Option<wl_seat::WlSeat>,
    manager: Option<ZwpVirtualKeyboardManagerV1>,
}

impl Dispatch<wl_registry::WlRegistry, ()> for State {
    fn event(
        s: &mut Self,
        reg: &wl_registry::WlRegistry,
        ev: wl_registry::Event,
        _: &(),
        _: &Connection,
        qh: &QueueHandle<Self>,
    ) {
        if let wl_registry::Event::Global { name, interface, .. } = ev {
            match interface.as_str() {
                "wl_seat" => s.seat = Some(reg.bind(name, 1, qh, ())),
                "zwp_virtual_keyboard_manager_v1" => s.manager = Some(reg.bind(name, 1, qh, ())),
                _ => {}
            }
        }
    }
}
delegate_noop!(State: ignore wl_seat::WlSeat);
delegate_noop!(State: ZwpVirtualKeyboardManagerV1);
delegate_noop!(State: ZwpVirtualKeyboardV1);

/// evdev code of a letter (qwerty rows), or a number as given.
fn code(k: &str) -> u32 {
    const ROWS: [(&str, u32); 3] = [("qwertyuiop", 16), ("asdfghjkl", 30), ("zxcvbnm", 44)];
    for (row, first) in ROWS {
        if let Some(i) = row.find(k).filter(|_| k.len() == 1) {
            return first + i as u32;
        }
    }
    k.parse().expect("KEY: a-z or an evdev code")
}

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let (mut mods, mut group) = (0u32, 0u32);
    for a in &args[..args.len() - 1] {
        match a.as_str() {
            "shift" => mods |= 1,
            "ctrl" => mods |= 4,
            "alt" => mods |= 8,
            "ru" => group = 1,
            o => panic!("unknown modifier {o}"),
        }
    }
    let key = code(args.last().expect("KEY"));
    let conn = Connection::connect_to_env().expect("WAYLAND_DISPLAY");
    let mut q = conn.new_event_queue();
    let qh = q.handle();
    conn.display().get_registry(&qh, ());
    let mut s = State::default();
    q.roundtrip(&mut s).unwrap();
    let m = s.manager.as_ref().expect("no zwp_virtual_keyboard_manager_v1");
    let kb = m.create_virtual_keyboard(s.seat.as_ref().expect("no seat"), &qh, ());
    let path = std::env::temp_dir().join(format!("vkey-{}.xkb", std::process::id()));
    std::fs::File::create(&path).unwrap().write_all(KEYMAP.as_bytes()).unwrap();
    let file = std::fs::File::open(&path).unwrap();
    kb.keymap(KeymapFormat::XkbV1.into(), file.as_fd(), KEYMAP.len() as u32);
    q.roundtrip(&mut s).unwrap();
    let _ = std::fs::remove_file(&path);
    let t = || std::time::UNIX_EPOCH.elapsed().unwrap().as_millis() as u32;
    let pause = || std::thread::sleep(std::time::Duration::from_millis(100));
    // Modifiers first, as a real keyboard's arrive before the key.
    kb.modifiers(mods, 0, 0, group);
    q.roundtrip(&mut s).unwrap();
    pause();
    kb.key(t(), key, KeyState::Pressed.into());
    q.roundtrip(&mut s).unwrap();
    pause();
    kb.key(t(), key, KeyState::Released.into());
    q.roundtrip(&mut s).unwrap();
    pause();
    kb.modifiers(0, 0, 0, group);
    q.roundtrip(&mut s).unwrap();
    std::thread::sleep(std::time::Duration::from_millis(300));
    kb.destroy();
    q.roundtrip(&mut s).unwrap();
}
