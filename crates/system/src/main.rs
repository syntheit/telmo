mod canvas;
mod effects;

fn main() {
    // STUB: the app shell replaces this.
    let mut logo = effects::Logo::place(effects::LogoKind::native(), 90, 21);
    let mut effect = effects::make(effects::NAMES[0], &logo);
    let mut canvas = canvas::Canvas::new(90, 21);
    canvas.clear();
    let mut f = effects::Frame {
        canvas: &mut canvas,
        logo: &mut logo,
        t: 0.0,
        dt: 0.016,
        busy: false,
        finished: false,
    };
    effect.frame(&mut f);
    let transition = effects::Transition::new(&logo);
    transition.apply(&mut canvas, &logo, 0.0);
    let _ = (
        transition.done(0.0),
        canvas.get(0, 0),
        logo.kind.other(),
        logo.masked(0, 0),
    );
}
