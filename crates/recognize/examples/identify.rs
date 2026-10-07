//! `identify <file.raw>`: raw f32le mono 16 kHz audio, e.g. from
//! `ffmpeg -i in.ogg -ac 1 -ar 16000 -f f32le out.raw`.

#[tokio::main(flavor = "current_thread")]
async fn main() {
    let Some(path) = std::env::args().nth(1) else {
        eprintln!("usage: identify <file.raw>");
        return;
    };
    let bytes = match std::fs::read(&path) {
        Ok(bytes) => bytes,
        Err(e) => return eprintln!("Couldn't read {path}: {e}."),
    };
    let samples: Vec<f32> = bytes
        .as_chunks::<4>()
        .0
        .iter()
        .map(|b| f32::from_le_bytes(*b))
        .collect();
    let sig = telmo_recognize::signature(&samples);
    println!("{} peaks", sig.bands.iter().map(Vec::len).sum::<usize>());
    match telmo_recognize::recognize(&sig).await {
        Ok(Some(track)) => println!("{track:#?}"),
        Ok(None) => println!("No match."),
        Err(e) => eprintln!("{e}"),
    }
}
