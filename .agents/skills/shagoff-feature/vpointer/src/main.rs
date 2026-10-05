//! `vpointer X Y [left|right|middle] [W H]`: move to (X, Y) of a W×H output (default 1036×530)
//! and click. `vpointer drag X1 Y1 X2 Y2 [W H]`: press at the first point, move to the second in
//! steps, release. For headless sway only (tests); it never touches a real session unless pointed at it.
use wayland_client::protocol::{wl_pointer::ButtonState, wl_registry, wl_seat};
use wayland_client::{Connection, Dispatch, QueueHandle, delegate_noop};
use wayland_protocols_wlr::virtual_pointer::v1::client::{
    zwlr_virtual_pointer_manager_v1::ZwlrVirtualPointerManagerV1,
    zwlr_virtual_pointer_v1::ZwlrVirtualPointerV1,
};

#[derive(Default)]
struct State {
    seat: Option<wl_seat::WlSeat>,
    manager: Option<ZwlrVirtualPointerManagerV1>,
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
                "zwlr_virtual_pointer_manager_v1" => s.manager = Some(reg.bind(name, 1, qh, ())),
                _ => {}
            }
        }
    }
}
delegate_noop!(State: ignore wl_seat::WlSeat);
delegate_noop!(State: ZwlrVirtualPointerManagerV1);
delegate_noop!(State: ZwlrVirtualPointerV1);

fn main() {
    let mut a: Vec<String> = std::env::args().collect();
    let drag = a.get(1).is_some_and(|s| s == "drag");
    if drag {
        a.remove(1);
    }
    let num = |i: usize, d: u32| a.get(i).and_then(|s| s.parse().ok()).unwrap_or(d);
    let (x, y, w, h) = if drag {
        (num(1, 0), num(2, 0), num(5, 1036), num(6, 530))
    } else {
        (num(1, 0), num(2, 0), num(4, 1036), num(5, 530))
    };
    let to = (num(3, 0), num(4, 0));
    let button = match a.get(3).map(String::as_str) {
        Some("right") => 0x111,
        Some("middle") => 0x112,
        Some("none") => 0,
        _ => 0x110,
    };
    let conn = Connection::connect_to_env().expect("WAYLAND_DISPLAY");
    let mut q = conn.new_event_queue();
    let qh = q.handle();
    conn.display().get_registry(&qh, ());
    let mut s = State::default();
    q.roundtrip(&mut s).unwrap();
    let m = s.manager.as_ref().expect("no zwlr_virtual_pointer_manager_v1");
    let p = m.create_virtual_pointer(s.seat.as_ref(), &qh, ());
    let t = || std::time::UNIX_EPOCH.elapsed().unwrap().as_millis() as u32;
    p.motion_absolute(t(), x, y, w, h);
    p.frame();
    q.roundtrip(&mut s).unwrap();
    std::thread::sleep(std::time::Duration::from_millis(150));
    if drag {
        p.button(t(), 0x110, ButtonState::Pressed);
        p.frame();
        q.roundtrip(&mut s).unwrap();
        for i in 1..=10 {
            let step = |a: u32, b: u32| (a as i64 + (b as i64 - a as i64) * i / 10) as u32;
            std::thread::sleep(std::time::Duration::from_millis(40));
            p.motion_absolute(t(), step(x, to.0), step(y, to.1), w, h);
            p.frame();
            q.roundtrip(&mut s).unwrap();
        }
        std::thread::sleep(std::time::Duration::from_millis(100));
        p.button(t(), 0x110, ButtonState::Released);
        p.frame();
        q.roundtrip(&mut s).unwrap();
    } else if button != 0 {
        p.button(t(), button, ButtonState::Pressed);
        p.frame();
        q.roundtrip(&mut s).unwrap();
        std::thread::sleep(std::time::Duration::from_millis(150));
        p.button(t(), button, ButtonState::Released);
        p.frame();
        q.roundtrip(&mut s).unwrap();
        // A real hand never holds still: libcosmic lays a context menu out on the next motion.
        std::thread::sleep(std::time::Duration::from_millis(100));
        p.motion_absolute(t(), x + 1, y + 1, w, h);
        p.frame();
        q.roundtrip(&mut s).unwrap();
    }
    // Keep the device a moment: destroying it at once can drop the last events.
    std::thread::sleep(std::time::Duration::from_millis(300));
    p.destroy();
    q.roundtrip(&mut s).unwrap();
}
