//! CRTSim Renderer as a frei0r filter: Kdenlive and other programs built on MLT run it over a
//! clip, as does FFmpeg's `frei0r` filter. See docs/EDITORS.md.
//!
//! frei0r hands a filter one frame at a time, with its time, and no way to ask for another. A
//! frame that follows the last, as an export's frames do, goes on with its glow, which decays by
//! how long the frame before lasted; one after a jump starts afresh, warmed up as a still is.

use crtsim_plugin::{Choice, Clip, Look, Timing, TUBES};
use std::ffi::{c_char, c_double, c_int, c_uint, c_void, CStr, CString};

/// The ABI of frei0r.h, version 1.2: a filter's description, its parameters' and the colour
/// model it reads and writes.
#[repr(C)]
pub struct PluginInfo {
    name: *const c_char,
    author: *const c_char,
    plugin_type: c_int,
    color_model: c_int,
    frei0r_version: c_int,
    major_version: c_int,
    minor_version: c_int,
    num_params: c_int,
    explanation: *const c_char,
}

#[repr(C)]
pub struct ParamInfo {
    name: *const c_char,
    kind: c_int,
    explanation: *const c_char,
}

const PLUGIN_TYPE_FILTER: c_int = 0;
/// Four bytes a pixel, red, green, blue and alpha, in that order in memory.
const COLOR_MODEL_RGBA8888: c_int = 1;
const PARAM_STRING: c_int = 4;

/// The filter's parameters, by index, each a string: its name, what it is, and its default.
/// Kdenlive's description of the effect lists their values (`kdenlive/crtsim.xml`).
const PARAMS: [(&CStr, &CStr); 4] = [
    (
        c"Look",
        c"An included look, by name: Super Win the Game, General image, NTSC 480i, and so on",
    ),
    (
        c"Look file",
        c"A look saved by the app or the command line as JSON, in place of Look; empty for none",
    ),
    (
        c"Tube",
        c"look, 4:3, 16:10 or 16:9, or 16x9 and the like: the tube's shape, the look's own by \
          default",
    ),
    (
        c"Timing",
        c"stable, alternating or off: glow between frames with the artifacts still or \
          alternating, or each frame on its own",
    ),
];

/// What the choice named `key` chooses, whatever its case.
fn chosen<T: Copy>(choices: &[Choice<T>], key: &str) -> Option<T> {
    choices
        .iter()
        .find(|choice| choice.key.eq_ignore_ascii_case(key))
        .map(|choice| choice.value)
}

/// The longest a frame may follow the last and still go on with its glow: a frame rate of at
/// least four a second.
const FOLLOWS_WITHIN: f64 = 0.25;

struct Instance {
    size: (u32, u32),
    look: Look,
    /// Each parameter's value as last set, which `f0r_get_param_value` hands out and must
    /// keep alive.
    values: [CString; 4],
    clip: Clip,
    /// The time and number of the last frame rendered, and how long the frames before it
    /// lasted.
    last: Option<(f64, i64)>,
    seconds: f64,
    /// Whether the host's times are known to be in seconds, as frei0r says and MLT gives,
    /// rather than the milliseconds FFmpeg's filter gives.
    in_seconds: bool,
    /// Whether a failure has been reported, so a clip of them reports one.
    reported: bool,
}

impl Instance {
    fn new(size: (u32, u32)) -> Self {
        let look = Look::default();
        let values = [
            CString::new(look.preset.as_str()).expect("a name with no NUL"),
            CString::default(),
            CString::new(TUBES[0].key).expect("no NUL"),
            CString::new(Timing::ALL[0].key).expect("no NUL"),
        ];
        Self {
            size,
            look,
            values,
            clip: Clip::default(),
            last: None,
            seconds: 1. / 30.,
            in_seconds: false,
            reported: false,
        }
    }

    /// Sets parameter `index` to `value`. A value it does not know leaves the parameter as it
    /// was.
    fn set(&mut self, index: usize, value: &str) {
        let known = match index {
            0 => {
                self.look.preset = value.to_owned();
                true
            }
            1 => {
                self.look.file = (!value.trim().is_empty()).then(|| value.trim().into());
                true
            }
            // FFmpeg's filter options take a colon as their own, so 16x9 or 16/9 will do too.
            2 => match chosen(&TUBES, &value.replace(['x', 'X', '/'], ":")) {
                Some(tube) => {
                    self.look.tube = tube;
                    true
                }
                None => false,
            },
            3 => match chosen(&Timing::ALL, value) {
                Some(timing) => {
                    self.look.timing = timing;
                    true
                }
                None => false,
            },
            _ => false,
        };
        if known {
            if let Ok(value) = CString::new(value) {
                self.values[index] = value;
            }
        }
    }

    /// Renders the frame at `time` from `input` into `output`, both RGBA. On a failure the
    /// frame passes through as it is, and the first failure of a run is reported.
    fn update(&mut self, time: f64, input: &[u8], output: &mut [u8]) {
        match self.render(time, input) {
            Ok(image) => {
                output.copy_from_slice(image.as_raw());
                self.reported = false;
            }
            Err(error) => {
                output.copy_from_slice(input);
                if !self.reported {
                    eprintln!("CRTSim Renderer: {error:#}");
                    self.reported = true;
                }
            }
        }
    }

    fn render(&mut self, time: f64, input: &[u8]) -> anyhow::Result<image::RgbaImage> {
        let picture = image::RgbaImage::from_raw(self.size.0, self.size.1, input.to_vec())
            .ok_or_else(|| anyhow::anyhow!("the frame is not {}x{}", self.size.0, self.size.1))?;
        let config = self.look.config(self.size)?;
        // A frame that follows the last goes on; any other is a clip's first, two frames on,
        // so it never seems to follow.
        let number = match self.last {
            Some((before, number)) => match self.follows(time - before) {
                Some(seconds) => {
                    self.seconds = seconds;
                    number + 1
                }
                None => number + 2,
            },
            None => 0,
        };
        let fps = 1. / self.seconds;
        let image = crtsim_plugin::with_renderer(|renderer| {
            self.clip.frame(
                renderer,
                &config,
                fps,
                number,
                number,
                &picture,
                |_| Ok(None),
                || false,
            )
        });
        if image.is_err() {
            self.clip.forget();
        }
        self.last = Some((time, number));
        image
    }

    /// How long the frame before lasted, in seconds, if a frame `gap` after it in the host's
    /// time follows it. A gap too short to be milliseconds says the host counts seconds; until
    /// one comes, a gap a frame could last in milliseconds is taken for them. A host that
    /// counts seconds and seeks before it plays carries the glow across that one seek.
    fn follows(&mut self, gap: f64) -> Option<f64> {
        if gap.is_nan() || gap <= 0. {
            return None;
        }
        self.in_seconds |= gap <= FOLLOWS_WITHIN;
        let seconds = if self.in_seconds { gap } else { gap / 1000. };
        (seconds <= FOLLOWS_WITHIN).then_some(seconds)
    }
}

#[no_mangle]
pub extern "C" fn f0r_init() -> c_int {
    1
}

#[no_mangle]
pub extern "C" fn f0r_deinit() {}

/// The filter's description, in strings that live as long as the library.
///
/// # Safety
/// `info` must point to a `f0r_plugin_info_t` to fill.
#[no_mangle]
pub unsafe extern "C" fn f0r_get_plugin_info(info: *mut PluginInfo) {
    let version = |part: &str| part.parse::<c_int>().unwrap_or(0);
    // SAFETY: the host passes a structure to fill, as frei0r.h says.
    unsafe {
        *info = PluginInfo {
            name: c"CRTSim Renderer".as_ptr(),
            author: c"aizumanga, after J. Kyle Pittman's CRTSim".as_ptr(),
            plugin_type: PLUGIN_TYPE_FILTER,
            color_model: COLOR_MODEL_RGBA8888,
            frei0r_version: 1,
            major_version: version(env!("CARGO_PKG_VERSION_MAJOR")),
            minor_version: version(env!("CARGO_PKG_VERSION_MINOR")),
            num_params: PARAMS.len() as c_int,
            explanation: c"A CRT simulation: composite artifacts, glow between frames, the curved \
                           glass and its bezel, shadow mask and bloom"
                .as_ptr(),
        };
    }
}

/// # Safety
/// `info` must point to a `f0r_param_info_t` to fill.
#[no_mangle]
pub unsafe extern "C" fn f0r_get_param_info(info: *mut ParamInfo, index: c_int) {
    let Some((name, explanation)) = usize::try_from(index).ok().and_then(|i| PARAMS.get(i)) else {
        return;
    };
    // SAFETY: the host passes a structure to fill, as frei0r.h says.
    unsafe {
        *info = ParamInfo {
            name: name.as_ptr(),
            kind: PARAM_STRING,
            explanation: explanation.as_ptr(),
        };
    }
}

#[no_mangle]
pub extern "C" fn f0r_construct(width: c_uint, height: c_uint) -> *mut c_void {
    Box::into_raw(Box::new(Instance::new((width, height)))).cast()
}

/// # Safety
/// `instance` must be one `f0r_construct` made, destructed once.
#[no_mangle]
pub unsafe extern "C" fn f0r_destruct(instance: *mut c_void) {
    if !instance.is_null() {
        // SAFETY: made by f0r_construct, and the host destructs each instance once.
        drop(unsafe { Box::from_raw(instance.cast::<Instance>()) });
    }
}

/// # Safety
/// `instance` must be live, and `param` point to an `f0r_param_string`: a C string.
#[no_mangle]
pub unsafe extern "C" fn f0r_set_param_value(
    instance: *mut c_void,
    param: *mut c_void,
    index: c_int,
) {
    // SAFETY: the host passes its instance and, for a string parameter, a pointer to a
    // NUL-terminated string.
    let (instance, value) = unsafe {
        let value = *param.cast::<*const c_char>();
        if instance.is_null() || value.is_null() {
            return;
        }
        (
            &mut *instance.cast::<Instance>(),
            CStr::from_ptr(value).to_string_lossy().into_owned(),
        )
    };
    if let Ok(index) = usize::try_from(index) {
        instance.set(index, &value);
    }
}

/// # Safety
/// `instance` must be live, and `param` point to an `f0r_param_string` to set. The string
/// handed out lives until the parameter is set again or the instance is destructed.
#[no_mangle]
pub unsafe extern "C" fn f0r_get_param_value(
    instance: *mut c_void,
    param: *mut c_void,
    index: c_int,
) {
    if instance.is_null() || param.is_null() {
        return;
    }
    // SAFETY: as above.
    let instance = unsafe { &*instance.cast::<Instance>() };
    if let Some(value) = usize::try_from(index)
        .ok()
        .and_then(|i| instance.values.get(i))
    {
        // SAFETY: the host passes where to put the string.
        unsafe { *param.cast::<*const c_char>() = value.as_ptr() };
    }
}

/// # Safety
/// `instance` must be live, and `inframe` and `outframe` each hold its width times height
/// pixels.
#[no_mangle]
pub unsafe extern "C" fn f0r_update(
    instance: *mut c_void,
    time: c_double,
    inframe: *const u32,
    outframe: *mut u32,
) {
    if instance.is_null() || inframe.is_null() || outframe.is_null() {
        return;
    }
    // SAFETY: the host passes its instance and two frames of the size it was made for.
    let (instance, input, output) = unsafe {
        let instance = &mut *instance.cast::<Instance>();
        let bytes = instance.size.0 as usize * instance.size.1 as usize * 4;
        (
            instance,
            std::slice::from_raw_parts(inframe.cast::<u8>(), bytes),
            std::slice::from_raw_parts_mut(outframe.cast::<u8>(), bytes),
        )
    };
    instance.update(time, input, output);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parameters_take_their_names_and_keep_what_they_do_not_know() {
        let mut instance = Instance::new((16, 8));
        instance.set(0, "NTSC 480i");
        instance.set(2, "16x10");
        assert_eq!(instance.look.tube, Some(16. / 10.));
        instance.set(2, "16:9");
        instance.set(3, "ALTERNATING");
        instance.set(3, "sideways");
        instance.set(1, "  ");
        assert_eq!(instance.look.preset, "NTSC 480i");
        assert_eq!(instance.look.tube, Some(16. / 9.));
        assert_eq!(instance.look.timing, Timing::Alternating);
        assert_eq!(instance.look.file, None);
        assert_eq!(instance.values[3].to_str().unwrap(), "ALTERNATING");
    }

    /// The keys of `choices`, as Kdenlive's description lists them.
    fn keys<T>(choices: &[Choice<T>]) -> String {
        choices
            .iter()
            .map(|choice| choice.key)
            .collect::<Vec<_>>()
            .join(";")
    }

    #[test]
    fn kdenlives_description_lists_the_filters_parameters_and_values() {
        let xml = include_str!("../kdenlive/crtsim.xml");
        let attribute = |parameter: &str, name: &str| -> String {
            let start = xml
                .find(&format!("name=\"{parameter}\""))
                .unwrap_or_else(|| panic!("no parameter {parameter}"));
            let tag = &xml[start..start + xml[start..].find('>').unwrap()];
            let value = tag.split(&format!("{name}=\"")).nth(1).unwrap_or_default();
            value[..value.find('"').unwrap_or(0)].to_owned()
        };
        let mut at = 0;
        for (name, _) in PARAMS {
            let name = name.to_str().unwrap();
            let found = xml.find(&format!("name=\"{name}\"")).unwrap();
            assert!(found > at, "{name} is in the filter's order");
            at = found;
        }
        assert_eq!(
            attribute("Look", "paramlist"),
            crtsim_plugin::preset_names().join(";")
        );
        assert_eq!(attribute("Look", "default"), crtsim_plugin::DEFAULT_PRESET);
        assert_eq!(attribute("Tube", "paramlist"), keys(&TUBES));
        assert_eq!(attribute("Timing", "paramlist"), keys(&Timing::ALL));
    }

    #[test]
    fn times_are_read_in_seconds_or_in_ffmpegs_milliseconds() {
        let mut ffmpeg = Instance::new((16, 8));
        assert_eq!(ffmpeg.follows(42.), Some(0.042));
        assert_eq!(ffmpeg.follows(41.), Some(0.041));
        assert_eq!(ffmpeg.follows(2000.), None, "a jump");
        assert_eq!(ffmpeg.follows(-42.), None, "back");
        let mut mlt = Instance::new((16, 8));
        assert_eq!(mlt.follows(0.04), Some(0.04));
        assert_eq!(mlt.follows(42.), None, "a jump, once seconds are known");
        assert_eq!(mlt.follows(0.), None, "the same frame again");
    }
}
