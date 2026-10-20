//! CRTSim Renderer as an OpenFX effect, for DaVinci Resolve, Natron, Nuke and the other hosts
//! of the standard: a filter over a clip, drawing each frame through the CRT at the clip's own
//! size. See docs/EDITORS.md.
//!
//! Editors render frames in any order. A frame that follows the last one rendered goes on with
//! its glow; any other asks the host for the frames before it, as far back as its glow reaches
//! (`crtsim_plugin::history`), and renders them first, so it comes out the same either way.

mod ffi;

use crtsim_plugin::{Clip, Look, Timing, TUBES};
use ffi::names::*;
use ffi::{ImageEffect, OfxStatus, PropertySet};
use image::RgbaImage;
use std::ffi::{c_char, c_int, c_void, CStr, CString};
use std::sync::{Mutex, OnceLock};

/// The effect's identifier: reverse domain name, then its own.
const IDENTIFIER: &CStr = c"io.github.aizumanga.CRTSimRenderer";

/// The host's suites, fetched when the effect loads.
struct Suites {
    host: *mut ffi::OfxHost,
    property: &'static ffi::PropertySuiteV1,
    effect: &'static ffi::ImageEffectSuiteV1,
    parameter: &'static ffi::ParameterSuiteV1,
    message: Option<&'static ffi::MessageSuiteV1>,
}

// SAFETY: the host's suites are tables of functions it says may be called from any thread
// rendering, and its handle lives as long as the library.
unsafe impl Send for Suites {}
unsafe impl Sync for Suites {}

static HOST: Mutex<usize> = Mutex::new(0);
static SUITES: OnceLock<Suites> = OnceLock::new();

fn suites() -> &'static Suites {
    SUITES.get().expect("the effect was loaded")
}

/// The effect as the host loads it.
struct Plugin(ffi::OfxPlugin);
// SAFETY: it holds pointers to static strings and functions only.
unsafe impl Sync for Plugin {}

static PLUGIN: Plugin = Plugin(ffi::OfxPlugin {
    plugin_api: IMAGE_EFFECT_PLUGIN_API.as_ptr(),
    api_version: 1,
    plugin_identifier: IDENTIFIER.as_ptr(),
    plugin_version_major: version(env!("CARGO_PKG_VERSION_MAJOR")),
    plugin_version_minor: version(env!("CARGO_PKG_VERSION_MINOR")),
    set_host,
    main_entry,
});

const fn version(text: &str) -> u32 {
    let bytes = text.as_bytes();
    let mut value = 0;
    let mut i = 0;
    while i < bytes.len() {
        value = value * 10 + (bytes[i] - b'0') as u32;
        i += 1;
    }
    value
}

#[no_mangle]
pub extern "C" fn OfxGetNumberOfPlugins() -> c_int {
    1
}

#[no_mangle]
pub extern "C" fn OfxGetPlugin(index: c_int) -> *const ffi::OfxPlugin {
    if index == 0 {
        &PLUGIN.0
    } else {
        std::ptr::null()
    }
}

unsafe extern "C" fn set_host(host: *mut ffi::OfxHost) {
    *HOST.lock().unwrap_or_else(|e| e.into_inner()) = host as usize;
}

unsafe extern "C" fn main_entry(
    action: *const c_char,
    handle: *const c_void,
    in_args: PropertySet,
    out_args: PropertySet,
) -> OfxStatus {
    if action.is_null() {
        return ffi::REPLY_DEFAULT;
    }
    // SAFETY: the host passes an action's name as a C string.
    let action = unsafe { CStr::from_ptr(action) };
    let effect = handle as ImageEffect;
    // A panic must not unwind into the host.
    std::panic::catch_unwind(|| act(action, effect, in_args, out_args)).unwrap_or(ffi::ERR_FATAL)
}

fn act(
    action: &CStr,
    effect: ImageEffect,
    in_args: PropertySet,
    out_args: PropertySet,
) -> OfxStatus {
    if action == ACTION_LOAD {
        return load();
    }
    if action == ACTION_UNLOAD {
        return ffi::OK;
    }
    if SUITES.get().is_none() {
        return ffi::ERR_MISSING_HOST_FEATURE;
    }
    let result = if action == ACTION_DESCRIBE {
        describe(effect)
    } else if action == ACTION_DESCRIBE_IN_CONTEXT {
        describe_in_context(effect)
    } else if action == ACTION_CREATE_INSTANCE {
        let instance = Box::into_raw(Box::new(Instance::default()));
        properties(effect).and_then(|p| p.set_pointer(PROP_INSTANCE_DATA, instance.cast()))
    } else if action == ACTION_DESTROY_INSTANCE {
        let instance = instance_pointer(effect);
        if !instance.is_null() {
            // SAFETY: made by the create action above, destroyed once.
            drop(unsafe { Box::from_raw(instance) });
        }
        Ok(())
    } else if action == ACTION_GET_CLIP_PREFERENCES {
        clip_preferences(effect, out_args)
    } else if action == ACTION_GET_FRAMES_NEEDED {
        return match frames_needed(effect, in_args, out_args) {
            Ok(()) => ffi::OK,
            Err(_) => ffi::REPLY_DEFAULT,
        };
    } else if action == ACTION_RENDER {
        render(effect, in_args).map_err(|error| {
            report(effect, &format!("{error:#}"));
            ffi::FAILED
        })
    } else {
        return ffi::REPLY_DEFAULT;
    };
    match result {
        Ok(()) => ffi::OK,
        Err(status) => status,
    }
}

/// What a step of an action failed with: the status the action returns.
type Step<T = ()> = Result<T, OfxStatus>;

fn check(status: OfxStatus) -> Step {
    if status == ffi::OK {
        Ok(())
    } else {
        Err(status)
    }
}

fn load() -> OfxStatus {
    let host = *HOST.lock().unwrap_or_else(|e| e.into_inner()) as *mut ffi::OfxHost;
    if host.is_null() {
        return ffi::ERR_MISSING_HOST_FEATURE;
    }
    // SAFETY: the host set itself before loading, and its suites live as long as the library.
    let suites = unsafe {
        let fetch =
            |name: &CStr, version| ((*host).fetch_suite)((*host).host, name.as_ptr(), version);
        let property = fetch(PROPERTY_SUITE, 1) as *const ffi::PropertySuiteV1;
        let effect = fetch(IMAGE_EFFECT_SUITE, 1) as *const ffi::ImageEffectSuiteV1;
        let parameter = fetch(PARAMETER_SUITE, 1) as *const ffi::ParameterSuiteV1;
        let message = fetch(MESSAGE_SUITE, 1) as *const ffi::MessageSuiteV1;
        if property.is_null() || effect.is_null() || parameter.is_null() {
            return ffi::ERR_MISSING_HOST_FEATURE;
        }
        Suites {
            host,
            property: &*property,
            effect: &*effect,
            parameter: &*parameter,
            message: message.as_ref(),
        }
    };
    let _ = SUITES.set(suites);
    ffi::OK
}

/// A property set, read and written through the host's property suite.
#[derive(Clone, Copy)]
struct Props(PropertySet);

impl Props {
    fn set_string(self, name: &CStr, value: &CStr) -> Step {
        // SAFETY: a property set the host handed over, and C strings that outlive the call.
        check(unsafe { (suites().property.set_string)(self.0, name.as_ptr(), 0, value.as_ptr()) })
    }

    fn set_strings(self, name: &CStr, values: &[&CStr]) -> Step {
        for (i, value) in values.iter().enumerate() {
            // SAFETY: as above.
            check(unsafe {
                (suites().property.set_string)(self.0, name.as_ptr(), i as c_int, value.as_ptr())
            })?;
        }
        Ok(())
    }

    fn set_int(self, name: &CStr, value: c_int) -> Step {
        // SAFETY: as above.
        check(unsafe { (suites().property.set_int)(self.0, name.as_ptr(), 0, value) })
    }

    fn set_doubles(self, name: &CStr, values: &[f64]) -> Step {
        // SAFETY: as above, with `values.len()` doubles.
        check(unsafe {
            (suites().property.set_double_n)(
                self.0,
                name.as_ptr(),
                values.len() as c_int,
                values.as_ptr(),
            )
        })
    }

    fn set_pointer(self, name: &CStr, value: *mut c_void) -> Step {
        // SAFETY: as above.
        check(unsafe { (suites().property.set_pointer)(self.0, name.as_ptr(), 0, value) })
    }

    fn string(self, name: &CStr) -> Step<String> {
        let mut value: *mut c_char = std::ptr::null_mut();
        // SAFETY: as above; the host owns the string it hands back.
        check(unsafe { (suites().property.get_string)(self.0, name.as_ptr(), 0, &mut value) })?;
        if value.is_null() {
            return Ok(String::new());
        }
        // SAFETY: a C string the host returned.
        Ok(unsafe { CStr::from_ptr(value) }
            .to_string_lossy()
            .into_owned())
    }

    fn int(self, name: &CStr) -> Step<c_int> {
        let mut value = 0;
        // SAFETY: as above.
        check(unsafe { (suites().property.get_int)(self.0, name.as_ptr(), 0, &mut value) })?;
        Ok(value)
    }

    fn ints<const N: usize>(self, name: &CStr) -> Step<[c_int; N]> {
        let mut values = [0; N];
        // SAFETY: as above, room for N.
        check(unsafe {
            (suites().property.get_int_n)(self.0, name.as_ptr(), N as c_int, values.as_mut_ptr())
        })?;
        Ok(values)
    }

    fn double(self, name: &CStr) -> Step<f64> {
        let mut value = 0.;
        // SAFETY: as above.
        check(unsafe { (suites().property.get_double)(self.0, name.as_ptr(), 0, &mut value) })?;
        Ok(value)
    }

    fn doubles<const N: usize>(self, name: &CStr) -> Step<[f64; N]> {
        let mut values = [0.; N];
        // SAFETY: as above, room for N.
        check(unsafe {
            (suites().property.get_double_n)(self.0, name.as_ptr(), N as c_int, values.as_mut_ptr())
        })?;
        Ok(values)
    }

    fn pointer(self, name: &CStr) -> Step<*mut c_void> {
        let mut value = std::ptr::null_mut();
        // SAFETY: as above.
        check(unsafe { (suites().property.get_pointer)(self.0, name.as_ptr(), 0, &mut value) })?;
        Ok(value)
    }
}

fn properties(effect: ImageEffect) -> Step<Props> {
    let mut set = std::ptr::null_mut();
    // SAFETY: the effect the host passed.
    check(unsafe { (suites().effect.get_property_set)(effect, &mut set) })?;
    Ok(Props(set))
}

/// The host's name, as it calls itself.
fn host_name() -> String {
    let host = suites().host;
    // SAFETY: the host's own property set.
    Props(unsafe { (*host).host })
        .string(PROP_NAME)
        .unwrap_or_default()
}

/// Whether the host hands effects scene-linear pictures by default, as compositors do.
fn linear_host() -> bool {
    let name = host_name().to_ascii_lowercase();
    name.contains("natron") || name.contains("nuke")
}

fn describe(effect: ImageEffect) -> Step {
    let props = properties(effect)?;
    props.set_string(PROP_LABEL, c"CRTSim Renderer")?;
    props.set_string(PLUGIN_GROUPING, c"CRTSim")?;
    props.set_string(
        PROP_PLUGIN_DESCRIPTION,
        c"J. Kyle Pittman's CRTSim over a clip: composite artifacts, glow between frames, the \
          curved glass and its bezel, shadow mask and bloom. Looks are the app's presets, or a \
          preset file it saved.",
    )?;
    props.set_strings(SUPPORTED_CONTEXTS, &[CONTEXT_FILTER, CONTEXT_GENERAL])?;
    props.set_strings(
        SUPPORTED_PIXEL_DEPTHS,
        &[BIT_DEPTH_BYTE, BIT_DEPTH_SHORT, BIT_DEPTH_FLOAT],
    )?;
    props.set_int(SUPPORTS_TILES, 0)?;
    props.set_int(SUPPORTS_MULTI_RESOLUTION, 0)?;
    props.set_int(TEMPORAL_CLIP_ACCESS, 1)?;
    props.set_string(RENDER_THREAD_SAFETY, RENDER_INSTANCE_SAFE)?;
    props.set_int(HOST_FRAME_THREADING, 0)?;
    props.set_int(FIELD_RENDER_TWICE_ALWAYS, 0)?;
    Ok(())
}

/// The effect's parameters, by name.
const LOOK: &CStr = c"look";
const LOOK_FILE: &CStr = c"lookFile";
const TUBE: &CStr = c"tube";
const TIMING: &CStr = c"timing";
const LINEAR: &CStr = c"linear";

fn describe_in_context(effect: ImageEffect) -> Step {
    for (name, temporal) in [(SOURCE_CLIP, 1), (OUTPUT_CLIP, 0)] {
        let mut clip = std::ptr::null_mut();
        // SAFETY: the effect descriptor the host passed.
        check(unsafe { (suites().effect.clip_define)(effect, name.as_ptr(), &mut clip) })?;
        let clip = Props(clip);
        clip.set_strings(SUPPORTED_COMPONENTS, &[COMPONENT_RGBA, COMPONENT_RGB])?;
        clip.set_int(SUPPORTS_TILES, 0)?;
        if temporal == 1 {
            clip.set_int(TEMPORAL_CLIP_ACCESS, 1)?;
        }
    }
    let mut params = std::ptr::null_mut();
    // SAFETY: as above.
    check(unsafe { (suites().effect.get_param_set)(effect, &mut params) })?;
    let define = |kind: &CStr, name: &CStr, label: &CStr, hint: &CStr| -> Step<Props> {
        let mut props = std::ptr::null_mut();
        // SAFETY: the descriptor's parameter set.
        check(unsafe {
            (suites().parameter.param_define)(params, kind.as_ptr(), name.as_ptr(), &mut props)
        })?;
        let props = Props(props);
        props.set_string(PROP_LABEL, label)?;
        props.set_string(PARAM_HINT, hint)?;
        props.set_int(PARAM_ANIMATES, 0)?;
        Ok(props)
    };
    let choice = |name, label, hint, options: &[CString], default: usize| -> Step {
        let props = define(PARAM_TYPE_CHOICE, name, label, hint)?;
        let options: Vec<&CStr> = options.iter().map(CString::as_c_str).collect();
        props.set_strings(PARAM_CHOICE_OPTION, &options)?;
        props.set_int(PARAM_DEFAULT, default as c_int)
    };
    let presets = crtsim_plugin::preset_names();
    let default = presets
        .iter()
        .position(|n| *n == crtsim_plugin::DEFAULT_PRESET)
        .unwrap_or(0);
    choice(
        LOOK,
        c"Look",
        c"An included look: the app's presets.",
        &c_strings(&presets),
        default,
    )?;
    let file = define(
        PARAM_TYPE_STRING,
        LOOK_FILE,
        c"Look file",
        c"A look the app or the command line saved as JSON, in place of Look.",
    )?;
    file.set_string(PARAM_STRING_MODE, PARAM_STRING_IS_FILE_PATH)?;
    file.set_int(PARAM_STRING_FILE_PATH_EXISTS, 1)?;
    file.set_string(PARAM_DEFAULT, c"")?;
    let tubes: Vec<&str> = TUBES.iter().map(|tube| tube.label).collect();
    choice(
        TUBE,
        c"Tube",
        c"The picture tube's shape: the look's own, or 4:3, 16:10 or 16:9.",
        &c_strings(&tubes),
        0,
    )?;
    let timings: Vec<&str> = Timing::ALL.iter().map(|timing| timing.label).collect();
    choice(
        TIMING,
        c"Timing",
        c"Whether each frame keeps the glow of the frames before it, and whether the composite \
          artifacts alternate between frames, as an NTSC set's did at 60 a second.",
        &c_strings(&timings),
        0,
    )?;
    let linear = define(
        PARAM_TYPE_BOOLEAN,
        LINEAR,
        c"Scene-linear input",
        c"The clip's values are scene-linear, as a compositor's are: encoded as sRGB for the CRT, \
          and decoded again after it. Off for gamma-encoded video, as an editor's timeline is.",
    )?;
    linear.set_int(PARAM_DEFAULT, c_int::from(linear_host()))?;
    Ok(())
}

fn c_strings(names: &[&str]) -> Vec<CString> {
    names
        .iter()
        .map(|n| CString::new(*n).expect("a name with no NUL"))
        .collect()
}

/// An effect's own: the clip it renders and how it last read its look.
#[derive(Default)]
struct Instance {
    clip: Mutex<Clip>,
}

fn instance_pointer(effect: ImageEffect) -> *mut Instance {
    properties(effect)
        .and_then(|p| p.pointer(PROP_INSTANCE_DATA))
        .unwrap_or(std::ptr::null_mut())
        .cast()
}

/// A parameter's value at `time`.
fn param(effect: ImageEffect, name: &CStr) -> Step<ffi::Param> {
    let mut params = std::ptr::null_mut();
    // SAFETY: the effect the host passed.
    check(unsafe { (suites().effect.get_param_set)(effect, &mut params) })?;
    let mut param = std::ptr::null_mut();
    // SAFETY: its parameter set, and a name it defined.
    check(unsafe {
        (suites().parameter.param_get_handle)(
            params,
            name.as_ptr(),
            &mut param,
            std::ptr::null_mut(),
        )
    })?;
    Ok(param)
}

fn int_at(effect: ImageEffect, name: &CStr, time: f64) -> Step<c_int> {
    let handle = param(effect, name)?;
    let mut value: c_int = 0;
    // SAFETY: a choice or boolean parameter, whose value at a time is an int.
    check(unsafe {
        (suites().parameter.param_get_value_at_time)(handle, time, &mut value as *mut c_int)
    })?;
    Ok(value)
}

fn string_at(effect: ImageEffect, name: &CStr, time: f64) -> Step<String> {
    let handle = param(effect, name)?;
    let mut value: *mut c_char = std::ptr::null_mut();
    // SAFETY: a string parameter, whose value at a time is a C string the host owns.
    check(unsafe {
        (suites().parameter.param_get_value_at_time)(handle, time, &mut value as *mut *mut c_char)
    })?;
    if value.is_null() {
        return Ok(String::new());
    }
    // SAFETY: a C string the host returned.
    Ok(unsafe { CStr::from_ptr(value) }
        .to_string_lossy()
        .into_owned())
}

/// The look and colour encoding the parameters name at `time`.
fn look_at(effect: ImageEffect, time: f64) -> Step<(Look, bool)> {
    let presets = crtsim_plugin::preset_names();
    let preset = presets
        .get(int_at(effect, LOOK, time)? as usize)
        .copied()
        .unwrap_or(crtsim_plugin::DEFAULT_PRESET);
    let file = string_at(effect, LOOK_FILE, time)?;
    let tube = TUBES
        .get(int_at(effect, TUBE, time)? as usize)
        .and_then(|tube| tube.value);
    let timing = Timing::ALL
        .get(int_at(effect, TIMING, time)? as usize)
        .map_or(Timing::Stable, |timing| timing.value);
    let look = Look {
        preset: preset.into(),
        file: (!file.trim().is_empty()).then(|| file.trim().into()),
        tube,
        timing,
    };
    Ok((look, int_at(effect, LINEAR, time)? != 0))
}

/// The source clip, and its property set.
fn source(effect: ImageEffect) -> Step<(ffi::ImageClip, Props)> {
    let mut clip = std::ptr::null_mut();
    let mut props = std::ptr::null_mut();
    // SAFETY: the effect the host passed, and a clip it defined.
    check(unsafe {
        (suites().effect.clip_get_handle)(effect, SOURCE_CLIP.as_ptr(), &mut clip, &mut props)
    })?;
    Ok((clip, Props(props)))
}

/// The CRT's picture is opaque, whatever its source's alpha; and with its artifacts
/// alternating, it changes from frame to frame even where the source holds still.
fn clip_preferences(effect: ImageEffect, out_args: PropertySet) -> Step {
    let out = Props(out_args);
    out.set_string(PREMULTIPLICATION, IMAGE_OPAQUE)?;
    // The parameters do not animate, so any time will do.
    let (look, _) = look_at(effect, 0.)?;
    out.set_int(
        FRAME_VARYING,
        c_int::from(look.timing == Timing::Alternating),
    )
}

/// The clip's frame rate, and its first frame, when the host says which it is.
fn timing_of(effect: ImageEffect, clip: Props) -> (f64, Option<i64>) {
    let fps = properties(effect)
        .and_then(|p| p.double(FRAME_RATE))
        .or_else(|_| clip.double(FRAME_RATE))
        .ok()
        .filter(|fps| *fps > 0.)
        .unwrap_or(24.);
    // A range of no length says nothing: Natron gives one for a sequence of files.
    let first = clip
        .doubles::<2>(FRAME_RANGE)
        .ok()
        .filter(|[first, last]| last > first)
        .map(|[first, _]| first.round() as i64);
    (fps, first)
}

/// Tells the host which of the source's frames a frame's render reads: those its glow needs,
/// back to the clip's first. A host that does not say where the clip starts is asked for the
/// frame alone; the render asks for the ones before it as it goes, and starts the glow after
/// any the host cannot give.
fn frames_needed(effect: ImageEffect, in_args: PropertySet, out_args: PropertySet) -> Step {
    let time = Props(in_args).double(PROP_TIME)?;
    let (look, _) = look_at(effect, time)?;
    let (_, clip) = source(effect)?;
    let (fps, first) = timing_of(effect, clip);
    let start = match first {
        Some(first) => {
            // The size is the frame's own; any will do to read the look's glow.
            let config = look.config((16, 16)).map_err(|_| ffi::FAILED)?;
            let history = i64::from(crtsim_plugin::history(&config, fps)) + 1;
            (time.round() as i64 - history).max(first) as f64
        }
        None => time,
    };
    Props(out_args).set_doubles(FRAMES_NEEDED_SOURCE, &[start, time])
}

/// An image the host lent, released when dropped.
struct Image {
    handle: PropertySet,
    data: *mut u8,
    /// x1, y1, x2, y2, in pixels, y up.
    bounds: [c_int; 4],
    row_bytes: isize,
    depth: Depth,
    channels: usize,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Depth {
    Byte,
    Short,
    Float,
}

impl Depth {
    fn bytes(self) -> usize {
        match self {
            Self::Byte => 1,
            Self::Short => 2,
            Self::Float => 4,
        }
    }
}

impl Drop for Image {
    fn drop(&mut self) {
        // SAFETY: an image the host lent, released once.
        unsafe { (suites().effect.clip_release_image)(self.handle) };
    }
}

impl Image {
    /// `clip`'s image at `time`, if the host has one.
    fn of(clip: ffi::ImageClip, time: f64) -> anyhow::Result<Option<Self>> {
        let mut handle = std::ptr::null_mut();
        // SAFETY: a clip of the effect rendering.
        let status =
            unsafe { (suites().effect.clip_get_image)(clip, time, std::ptr::null(), &mut handle) };
        if status != ffi::OK || handle.is_null() {
            return Ok(None);
        }
        let props = Props(handle);
        let failed = |what: &str| anyhow::anyhow!("the host's image has no {what}");
        let mut image = Image {
            handle,
            data: std::ptr::null_mut(),
            bounds: [0; 4],
            row_bytes: 0,
            depth: Depth::Byte,
            channels: 4,
        };
        image.data = props
            .pointer(IMAGE_DATA)
            .map_err(|_| failed("pixels"))?
            .cast();
        image.bounds = props
            .ints::<4>(IMAGE_BOUNDS)
            .map_err(|_| failed("bounds"))?;
        image.row_bytes = props
            .int(IMAGE_ROW_BYTES)
            .map_err(|_| failed("row length"))? as isize;
        let depth = props
            .string(PIXEL_DEPTH)
            .map_err(|_| failed("pixel depth"))?;
        image.depth = match depth.as_bytes() {
            b"OfxBitDepthByte" => Depth::Byte,
            b"OfxBitDepthShort" => Depth::Short,
            b"OfxBitDepthFloat" => Depth::Float,
            _ => anyhow::bail!("the host's image is {depth}, which the effect does not read"),
        };
        let components = props.string(COMPONENTS).map_err(|_| failed("components"))?;
        image.channels = match components.as_bytes() {
            b"OfxImageComponentRGBA" => 4,
            b"OfxImageComponentRGB" => 3,
            _ => anyhow::bail!("the host's image is {components}, which the effect does not read"),
        };
        anyhow::ensure!(!image.data.is_null(), "the host's image has no pixels");
        Ok(Some(image))
    }

    fn size(&self) -> (u32, u32) {
        let [x1, y1, x2, y2] = self.bounds;
        ((x2 - x1).max(0) as u32, (y2 - y1).max(0) as u32)
    }

    /// The bytes of pixel (x, y) of the image, counted from its bounds' corner.
    fn pixel(&self, x: usize, y: usize) -> *mut u8 {
        let offset =
            y as isize * self.row_bytes + (x * self.channels * self.depth.bytes()) as isize;
        // SAFETY: within the image the host lent, by its bounds and row length.
        unsafe { self.data.offset(offset) }
    }

    /// The image as the CRT reads it: 8-bit, top row first, gamma-encoded, opaque, its colour
    /// as it shows over black.
    fn picture(&self, linear: bool, unpremultiplied: bool) -> RgbaImage {
        let (width, height) = self.size();
        RgbaImage::from_fn(width, height, |x, row| {
            let y = (height - 1 - row) as usize;
            let mut value = [0_f32; 4];
            value[3] = 1.;
            let pixel = self.pixel(x as usize, y);
            for (c, v) in value.iter_mut().enumerate().take(self.channels) {
                // SAFETY: a channel of a pixel within the image.
                *v = unsafe {
                    match self.depth {
                        Depth::Byte => f32::from(*pixel.add(c)) / 255.,
                        Depth::Short => {
                            f32::from((pixel.add(c * 2) as *const u16).read_unaligned()) / 65535.
                        }
                        Depth::Float => (pixel.add(c * 4) as *const f32).read_unaligned(),
                    }
                };
            }
            let alpha = value[3].clamp(0., 1.);
            image::Rgba(std::array::from_fn(|c| {
                if c == 3 {
                    return 255;
                }
                let mut v = value[c];
                if unpremultiplied {
                    v *= alpha;
                }
                if linear {
                    v = encode(v);
                }
                (v.clamp(0., 1.) * 255.).round() as u8
            }))
        })
    }

    /// Writes `crt` into the image within `window`, where the source's `bounds` lie over it.
    fn write(&self, crt: &RgbaImage, source: [c_int; 4], window: [c_int; 4], linear: bool) {
        let [x1, y1, x2, y2] = self.bounds;
        let (from_x, to_x) = (window[0].max(x1), window[2].min(x2));
        let (from_y, to_y) = (window[1].max(y1), window[3].min(y2));
        for y in from_y..to_y {
            for x in from_x..to_x {
                let (cx, cy) = (x - source[0], source[3] - 1 - y);
                let rgb = if cx >= 0
                    && cy >= 0
                    && (cx as u32) < crt.width()
                    && (cy as u32) < crt.height()
                {
                    let p = crt.get_pixel(cx as u32, cy as u32);
                    [p[0], p[1], p[2]]
                } else {
                    [0; 3]
                };
                let pixel = self.pixel((x - x1) as usize, (y - y1) as usize);
                for (c, &value) in rgb.iter().chain(&[255]).enumerate().take(self.channels) {
                    let mut v = f32::from(value) / 255.;
                    if linear && c < 3 {
                        v = decode(v);
                    }
                    // SAFETY: a channel of a pixel within the image the host lent to write.
                    unsafe {
                        match self.depth {
                            Depth::Byte => *pixel.add(c) = (v * 255.).round() as u8,
                            Depth::Short => (pixel.add(c * 2) as *mut u16)
                                .write_unaligned((v * 65535.).round() as u16),
                            Depth::Float => (pixel.add(c * 4) as *mut f32).write_unaligned(v),
                        }
                    }
                }
            }
        }
    }
}

fn encode(v: f32) -> f32 {
    if v <= 0.003_130_8 {
        v * 12.92
    } else {
        1.055 * v.powf(1. / 2.4) - 0.055
    }
}

fn decode(v: f32) -> f32 {
    if v <= 0.040_45 {
        v / 12.92
    } else {
        ((v + 0.055) / 1.055).powf(2.4)
    }
}

fn render(effect: ImageEffect, in_args: PropertySet) -> anyhow::Result<()> {
    let args = Props(in_args);
    let failed = |what: &str| anyhow::anyhow!("the host did not say {what}");
    let time = args.double(PROP_TIME).map_err(|_| failed("the time"))?;
    let window = args
        .ints::<4>(RENDER_WINDOW)
        .map_err(|_| failed("the render window"))?;
    let (look, linear) = look_at(effect, time).map_err(|_| failed("its parameters"))?;
    let (clip, clip_props) = source(effect).map_err(|_| failed("its source"))?;
    let unpremultiplied = clip_props
        .string(PREMULTIPLICATION)
        .is_ok_and(|p| p.as_bytes() == IMAGE_UNPREMULTIPLIED.to_bytes());
    let (fps, first) = timing_of(effect, clip_props);
    let mut output_clip = std::ptr::null_mut();
    // SAFETY: the effect rendering, and a clip it defined.
    check(unsafe {
        (suites().effect.clip_get_handle)(
            effect,
            OUTPUT_CLIP.as_ptr(),
            &mut output_clip,
            std::ptr::null_mut(),
        )
    })
    .map_err(|_| failed("its output"))?;
    let source_image = Image::of(clip, time)?.ok_or_else(|| failed("its source's frame"))?;
    let output = Image::of(output_clip, time)?.ok_or_else(|| failed("its output's frame"))?;
    let picture = source_image.picture(linear, unpremultiplied);
    let config = look.config(picture.dimensions())?;
    let instance = instance_pointer(effect);
    anyhow::ensure!(!instance.is_null(), "the effect has no instance");
    // SAFETY: made by the create action, and the host renders an instance once at a time.
    let instance = unsafe { &*instance };
    let mut clip_state = instance.clip.lock().unwrap_or_else(|e| e.into_inner());
    let size = picture.dimensions();
    let fetch = |frame: i64| -> anyhow::Result<Option<RgbaImage>> {
        Ok(Image::of(clip, frame as f64)?
            .filter(|image| image.size() == size)
            .map(|image| image.picture(linear, unpremultiplied)))
    };
    // SAFETY: the effect rendering.
    let cancelled = || unsafe { (suites().effect.abort)(effect) } != 0;
    let crt = crtsim_plugin::with_renderer(|renderer| {
        clip_state.frame(
            renderer,
            &config,
            fps,
            time.round() as i64,
            // Where a host does not say, frames alternate in step with frame 0.
            first.unwrap_or(0),
            &picture,
            fetch,
            cancelled,
        )
    });
    let crt = match crt {
        Ok(crt) => crt,
        Err(error) => {
            clip_state.forget();
            return Err(error);
        }
    };
    output.write(&crt, source_image.bounds, window, linear);
    Ok(())
}

/// Tells the person why a frame failed, as the host shows an effect's errors.
fn report(effect: ImageEffect, message: &str) {
    eprintln!("CRTSim Renderer: {message}");
    let (Some(suite), Ok(text)) = (suites().message, CString::new(message)) else {
        return;
    };
    // SAFETY: the effect, and a format that takes one C string.
    unsafe {
        (suite.message)(
            effect,
            MESSAGE_ERROR.as_ptr(),
            std::ptr::null(),
            c"%s".as_ptr(),
            text.as_ptr(),
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn srgb_round_trips_and_the_version_is_the_crates() {
        for v in [0., 0.001, 0.2, 0.5, 1.] {
            assert!((decode(encode(v)) - v).abs() < 1e-5);
        }
        assert_eq!(
            PLUGIN.0.plugin_version_major,
            env!("CARGO_PKG_VERSION_MAJOR").parse::<u32>().unwrap()
        );
        assert!(OfxGetPlugin(1).is_null());
        assert_eq!(OfxGetNumberOfPlugins(), 1);
    }
}
