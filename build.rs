use std::env;
use std::fs;
use std::path::PathBuf;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    slint_build::compile("ui/app.slint")?;
    println!("cargo:rerun-if-changed=assets/imggen-icon.svg");

    if env::var("CARGO_CFG_TARGET_OS").as_deref() == Ok("windows")
        && env::var("CARGO_CFG_TARGET_ENV").as_deref() == Ok("msvc")
    {
        let out_dir = PathBuf::from(env::var_os("OUT_DIR").ok_or("OUT_DIR is missing")?);
        let icon_path = out_dir.join("imggen-icon.ico");
        write_icon(&icon_path)?;

        let mut resource = winresource::WindowsResource::new();
        resource.set_icon(icon_path.to_str().ok_or("icon path is not valid UTF-8")?);
        resource.compile()?;
    }

    Ok(())
}

fn write_icon(path: &PathBuf) -> Result<(), Box<dyn std::error::Error>> {
    let svg = fs::read("assets/imggen-icon.svg")?;
    let tree = resvg::usvg::Tree::from_data(&svg, &resvg::usvg::Options::default())?;
    let source_size = tree.size();
    let mut frames = Vec::new();

    for size in [16, 32, 48, 64, 128, 256] {
        let size = size as u32;
        let scale = size as f32 / source_size.width().max(source_size.height());
        let mut pixmap =
            resvg::tiny_skia::Pixmap::new(size, size).ok_or("icon pixmap allocation failed")?;
        resvg::render(
            &tree,
            resvg::tiny_skia::Transform::from_scale(scale, scale),
            &mut pixmap.as_mut(),
        );
        frames.push(image::codecs::ico::IcoFrame::as_png(
            pixmap.data(),
            size,
            size,
            image::ExtendedColorType::Rgba8,
        )?);
    }

    let mut ico = Vec::new();
    image::codecs::ico::IcoEncoder::new(&mut ico).encode_images(&frames)?;
    fs::write(path, ico)?;
    Ok(())
}
