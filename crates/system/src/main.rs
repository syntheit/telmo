mod canvas;
mod effects;

fn main() {
    // STUB: the app shell replaces this.
    let logo = effects::Logo::place(effects::LogoKind::native(), 90, 21);
    let _ = effects::make(effects::NAMES[0], &logo);
    let _ = effects::Transition::new(&logo).done(0.0);
    let _ = (canvas::Canvas::new(1, 1).get(0, 0), logo.kind.other(), logo.masked(0, 0));
}
