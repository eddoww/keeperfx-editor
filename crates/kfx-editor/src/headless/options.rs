//! The command line of the screenshot mode (`--screenshot OUT.png --map MAP [options]`).
use super::*;

#[derive(Debug, Clone)]
pub struct Options {
    pub out: PathBuf,
    pub map: PathBuf,
    pub size: (u32, u32),
    pub camera: Option<CameraSpec>,
    pub perspective: bool,
    pub viewport_only: bool,
    pub pointer: Option<(f32, f32)>,
    pub frame: u32,
    pub bench: u32,
    pub slab_image: Option<PathBuf>,
    pub show_browser: bool,
    pub browser_filter: String,
    pub rooms_palette: bool,
    pub actions: Option<PathBuf>,
    /// `--record DIR|-`: write a frame for every simulated frame of the recording actions.
    pub record: Option<String>,
    /// `--record-captions FILE`: the captions of a recording, as JSON.
    pub record_captions: Option<PathBuf>,
    /// `--fps N` of a recording (default 30).
    pub fps: f32,
    /// `--forbid-text TEXT` (repeatable): fail when a frame or screenshot shows this text.
    pub forbid: Vec<String>,
}

pub(super) fn floats(s: &str, n: usize, what: &str) -> Result<Vec<f32>, String> {
    let v: Vec<f32> = s
        .split([',', 'x'])
        .map(|t| t.trim().parse::<f32>())
        .collect::<Result<_, _>>()
        .map_err(|_| format!("{what}: bad number in {s:?}"))?;
    if v.len() != n {
        return Err(format!("{what}: expected {n} values, got {s:?}"));
    }
    Ok(v)
}

impl Options {
    pub fn parse(args: &[String]) -> Result<Options, String> {
        let mut o = Options {
            out: PathBuf::new(),
            map: PathBuf::new(),
            size: (1600, 1000),
            camera: None,
            perspective: false,
            viewport_only: false,
            pointer: None,
            frame: 0,
            bench: 0,
            slab_image: None,
            show_browser: false,
            browser_filter: String::new(),
            rooms_palette: false,
            actions: None,
            record: None,
            record_captions: None,
            fps: 30.0,
            forbid: vec![],
        };
        let mut it = args.iter();
        while let Some(a) = it.next() {
            let mut val = || {
                it.next()
                    .cloned()
                    .ok_or_else(|| format!("{a} needs a value"))
            };
            match a.as_str() {
                "--screenshot" => o.out = PathBuf::from(val()?),
                "--map" => o.map = PathBuf::from(val()?),
                "--size" => {
                    let v = floats(&val()?, 2, "--size")?;
                    o.size = (v[0].max(16.0) as u32, v[1].max(16.0) as u32);
                }
                "--camera" => {
                    let v = floats(&val()?, 5, "--camera")?;
                    o.camera = Some(CameraSpec {
                        x: v[0],
                        y: v[1],
                        zoom_pct: v[2],
                        yaw_deg: v[3],
                        pitch_deg: v[4],
                    });
                }
                "--pointer" => {
                    let v = floats(&val()?, 2, "--pointer")?;
                    o.pointer = Some((v[0], v[1]));
                }
                "--frame" => o.frame = val()?.parse().map_err(|_| "--frame: not a number")?,
                "--bench" => o.bench = val()?.parse().map_err(|_| "--bench: not a number")?,
                "--perspective" => o.perspective = true,
                "--viewport-only" => o.viewport_only = true,
                "--show-browser" => o.show_browser = true,
                "--browser-filter" => {
                    o.show_browser = true;
                    o.browser_filter = val()?;
                }
                "--rooms-palette" => o.rooms_palette = true,
                "--slab-image" => o.slab_image = Some(PathBuf::from(val()?)),
                "--actions" => o.actions = Some(PathBuf::from(val()?)),
                "--record" => o.record = Some(val()?),
                "--record-captions" => o.record_captions = Some(PathBuf::from(val()?)),
                "--fps" => {
                    o.fps = val()?
                        .parse::<f32>()
                        .map_err(|_| "--fps: not a number")?
                        .clamp(1.0, 240.0)
                }
                "--forbid-text" => o.forbid.push(val()?),
                other => return Err(format!("unknown argument {other:?} (see --help)")),
            }
        }
        if o.out.as_os_str().is_empty() || o.map.as_os_str().is_empty() {
            return Err("--screenshot needs OUT.png and --map MAP".into());
        }
        Ok(o)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_options() {
        let a: Vec<String> = "--screenshot /tmp/a.png --map m/map00001.slb --size 800x600 \
        --camera 10,20,150,45,-46.8 --viewport-only"
            .split(' ')
            .map(String::from)
            .collect();
        let o = Options::parse(&a).unwrap();
        assert_eq!(o.size, (800, 600));
        assert!(o.viewport_only);
        let c = o.camera.unwrap();
        assert_eq!(
            (c.x, c.y, c.zoom_pct, c.yaw_deg, c.pitch_deg),
            (10.0, 20.0, 150.0, 45.0, -46.8)
        );
        assert!(Options::parse(&["--screenshot".into(), "x.png".into()]).is_err());
        assert!(Options::parse(&[
            "--screenshot".into(),
            "x.png".into(),
            "--map".into(),
            "m".into(),
            "--bogus".into()
        ])
        .is_err());
    }
}
