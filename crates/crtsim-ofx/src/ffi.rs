//! The parts of the OpenFX C API (ofxCore.h, ofxProperty.h, ofxImageEffect.h, ofxParam.h and
//! ofxMessage.h, release 1.4 and later) the effect uses, as they are laid out in C.

#![allow(non_camel_case_types, dead_code)]

use std::ffi::{c_char, c_double, c_int, c_uint, c_void};

pub type OfxStatus = c_int;
pub type OfxTime = c_double;
pub type PropertySet = *mut c_void;
pub type ImageEffect = *mut c_void;
pub type ImageClip = *mut c_void;
pub type ParamSet = *mut c_void;
pub type Param = *mut c_void;

pub const OK: OfxStatus = 0;
pub const FAILED: OfxStatus = 1;
pub const ERR_FATAL: OfxStatus = 2;
pub const ERR_MISSING_HOST_FEATURE: OfxStatus = 4;
pub const ERR_UNSUPPORTED: OfxStatus = 5;
pub const ERR_BAD_HANDLE: OfxStatus = 9;
pub const REPLY_DEFAULT: OfxStatus = 14;

#[repr(C)]
pub struct OfxHost {
    pub host: PropertySet,
    pub fetch_suite: unsafe extern "C" fn(
        host: PropertySet,
        name: *const c_char,
        version: c_int,
    ) -> *const c_void,
}

pub type EntryPoint = unsafe extern "C" fn(
    action: *const c_char,
    handle: *const c_void,
    in_args: PropertySet,
    out_args: PropertySet,
) -> OfxStatus;

#[repr(C)]
pub struct OfxPlugin {
    pub plugin_api: *const c_char,
    pub api_version: c_int,
    pub plugin_identifier: *const c_char,
    pub plugin_version_major: c_uint,
    pub plugin_version_minor: c_uint,
    pub set_host: unsafe extern "C" fn(host: *mut OfxHost),
    pub main_entry: EntryPoint,
}

#[repr(C)]
#[derive(Clone, Copy, Debug, Default)]
pub struct RectD {
    pub x1: c_double,
    pub y1: c_double,
    pub x2: c_double,
    pub y2: c_double,
}

type Props = PropertySet;
type Name = *const c_char;

#[repr(C)]
pub struct PropertySuiteV1 {
    pub set_pointer: unsafe extern "C" fn(Props, Name, c_int, *mut c_void) -> OfxStatus,
    pub set_string: unsafe extern "C" fn(Props, Name, c_int, *const c_char) -> OfxStatus,
    pub set_double: unsafe extern "C" fn(Props, Name, c_int, c_double) -> OfxStatus,
    pub set_int: unsafe extern "C" fn(Props, Name, c_int, c_int) -> OfxStatus,
    pub set_pointer_n: unsafe extern "C" fn(Props, Name, c_int, *const *mut c_void) -> OfxStatus,
    pub set_string_n: unsafe extern "C" fn(Props, Name, c_int, *const *const c_char) -> OfxStatus,
    pub set_double_n: unsafe extern "C" fn(Props, Name, c_int, *const c_double) -> OfxStatus,
    pub set_int_n: unsafe extern "C" fn(Props, Name, c_int, *const c_int) -> OfxStatus,
    pub get_pointer: unsafe extern "C" fn(Props, Name, c_int, *mut *mut c_void) -> OfxStatus,
    pub get_string: unsafe extern "C" fn(Props, Name, c_int, *mut *mut c_char) -> OfxStatus,
    pub get_double: unsafe extern "C" fn(Props, Name, c_int, *mut c_double) -> OfxStatus,
    pub get_int: unsafe extern "C" fn(Props, Name, c_int, *mut c_int) -> OfxStatus,
    pub get_pointer_n: unsafe extern "C" fn(Props, Name, c_int, *mut *mut c_void) -> OfxStatus,
    pub get_string_n: unsafe extern "C" fn(Props, Name, c_int, *mut *mut c_char) -> OfxStatus,
    pub get_double_n: unsafe extern "C" fn(Props, Name, c_int, *mut c_double) -> OfxStatus,
    pub get_int_n: unsafe extern "C" fn(Props, Name, c_int, *mut c_int) -> OfxStatus,
    pub reset: unsafe extern "C" fn(Props, Name) -> OfxStatus,
    pub get_dimension: unsafe extern "C" fn(Props, Name, *mut c_int) -> OfxStatus,
}

#[repr(C)]
pub struct ImageEffectSuiteV1 {
    pub get_property_set: unsafe extern "C" fn(ImageEffect, *mut PropertySet) -> OfxStatus,
    pub get_param_set: unsafe extern "C" fn(ImageEffect, *mut ParamSet) -> OfxStatus,
    pub clip_define: unsafe extern "C" fn(ImageEffect, Name, *mut PropertySet) -> OfxStatus,
    pub clip_get_handle:
        unsafe extern "C" fn(ImageEffect, Name, *mut ImageClip, *mut PropertySet) -> OfxStatus,
    pub clip_get_property_set: unsafe extern "C" fn(ImageClip, *mut PropertySet) -> OfxStatus,
    pub clip_get_image:
        unsafe extern "C" fn(ImageClip, OfxTime, *const RectD, *mut PropertySet) -> OfxStatus,
    pub clip_release_image: unsafe extern "C" fn(PropertySet) -> OfxStatus,
    pub clip_get_region_of_definition:
        unsafe extern "C" fn(ImageClip, OfxTime, *mut RectD) -> OfxStatus,
    pub abort: unsafe extern "C" fn(ImageEffect) -> c_int,
    pub image_memory_alloc: unsafe extern "C" fn(ImageEffect, usize, *mut *mut c_void) -> OfxStatus,
    pub image_memory_free: unsafe extern "C" fn(*mut c_void) -> OfxStatus,
    pub image_memory_lock: unsafe extern "C" fn(*mut c_void, *mut *mut c_void) -> OfxStatus,
    pub image_memory_unlock: unsafe extern "C" fn(*mut c_void) -> OfxStatus,
}

#[repr(C)]
pub struct ParameterSuiteV1 {
    pub param_define: unsafe extern "C" fn(ParamSet, Name, Name, *mut PropertySet) -> OfxStatus,
    pub param_get_handle:
        unsafe extern "C" fn(ParamSet, Name, *mut Param, *mut PropertySet) -> OfxStatus,
    pub param_set_get_property_set: unsafe extern "C" fn(ParamSet, *mut PropertySet) -> OfxStatus,
    pub param_get_property_set: unsafe extern "C" fn(Param, *mut PropertySet) -> OfxStatus,
    pub param_get_value: unsafe extern "C" fn(Param, ...) -> OfxStatus,
    pub param_get_value_at_time: unsafe extern "C" fn(Param, OfxTime, ...) -> OfxStatus,
    pub param_get_derivative: unsafe extern "C" fn(Param, OfxTime, ...) -> OfxStatus,
    pub param_get_integral: unsafe extern "C" fn(Param, OfxTime, OfxTime, ...) -> OfxStatus,
    pub param_set_value: unsafe extern "C" fn(Param, ...) -> OfxStatus,
    pub param_set_value_at_time: unsafe extern "C" fn(Param, OfxTime, ...) -> OfxStatus,
    pub param_get_num_keys: unsafe extern "C" fn(Param, *mut c_uint) -> OfxStatus,
    pub param_get_key_time: unsafe extern "C" fn(Param, c_uint, *mut OfxTime) -> OfxStatus,
    pub param_get_key_index: unsafe extern "C" fn(Param, OfxTime, c_int, *mut c_int) -> OfxStatus,
    pub param_delete_key: unsafe extern "C" fn(Param, OfxTime) -> OfxStatus,
    pub param_delete_all_keys: unsafe extern "C" fn(Param) -> OfxStatus,
    pub param_copy: unsafe extern "C" fn(Param, Param, OfxTime, *const c_void) -> OfxStatus,
    pub param_edit_begin: unsafe extern "C" fn(ParamSet, Name) -> OfxStatus,
    pub param_edit_end: unsafe extern "C" fn(ParamSet) -> OfxStatus,
}

#[repr(C)]
pub struct MessageSuiteV1 {
    pub message: unsafe extern "C" fn(*mut c_void, Name, Name, Name, ...) -> OfxStatus,
}

/// The strings the API names actions, properties and their values with.
pub mod names {
    use std::ffi::CStr;

    pub const IMAGE_EFFECT_PLUGIN_API: &CStr = c"OfxImageEffectPluginAPI";
    pub const PROPERTY_SUITE: &CStr = c"OfxPropertySuite";
    pub const IMAGE_EFFECT_SUITE: &CStr = c"OfxImageEffectSuite";
    pub const PARAMETER_SUITE: &CStr = c"OfxParameterSuite";
    pub const MESSAGE_SUITE: &CStr = c"OfxMessageSuite";

    pub const ACTION_LOAD: &CStr = c"OfxActionLoad";
    pub const ACTION_UNLOAD: &CStr = c"OfxActionUnload";
    pub const ACTION_DESCRIBE: &CStr = c"OfxActionDescribe";
    pub const ACTION_CREATE_INSTANCE: &CStr = c"OfxActionCreateInstance";
    pub const ACTION_DESTROY_INSTANCE: &CStr = c"OfxActionDestroyInstance";
    pub const ACTION_DESCRIBE_IN_CONTEXT: &CStr = c"OfxImageEffectActionDescribeInContext";
    pub const ACTION_RENDER: &CStr = c"OfxImageEffectActionRender";
    pub const ACTION_GET_FRAMES_NEEDED: &CStr = c"OfxImageEffectActionGetFramesNeeded";
    pub const ACTION_GET_CLIP_PREFERENCES: &CStr = c"OfxImageEffectActionGetClipPreferences";

    pub const PROP_NAME: &CStr = c"OfxPropName";
    pub const PROP_LABEL: &CStr = c"OfxPropLabel";
    pub const PROP_PLUGIN_DESCRIPTION: &CStr = c"OfxPropPluginDescription";
    pub const PROP_INSTANCE_DATA: &CStr = c"OfxPropInstanceData";
    pub const PROP_TIME: &CStr = c"OfxPropTime";
    pub const PLUGIN_GROUPING: &CStr = c"OfxImageEffectPluginPropGrouping";
    pub const SUPPORTED_CONTEXTS: &CStr = c"OfxImageEffectPropSupportedContexts";
    pub const CONTEXT_FILTER: &CStr = c"OfxImageEffectContextFilter";
    pub const CONTEXT_GENERAL: &CStr = c"OfxImageEffectContextGeneral";
    pub const SUPPORTED_PIXEL_DEPTHS: &CStr = c"OfxImageEffectPropSupportedPixelDepths";
    pub const BIT_DEPTH_BYTE: &CStr = c"OfxBitDepthByte";
    pub const BIT_DEPTH_SHORT: &CStr = c"OfxBitDepthShort";
    pub const BIT_DEPTH_FLOAT: &CStr = c"OfxBitDepthFloat";
    pub const SUPPORTED_COMPONENTS: &CStr = c"OfxImageEffectPropSupportedComponents";
    pub const COMPONENT_RGBA: &CStr = c"OfxImageComponentRGBA";
    pub const COMPONENT_RGB: &CStr = c"OfxImageComponentRGB";
    pub const SUPPORTS_TILES: &CStr = c"OfxImageEffectPropSupportsTiles";
    pub const SUPPORTS_MULTI_RESOLUTION: &CStr = c"OfxImageEffectPropSupportsMultiResolution";
    pub const TEMPORAL_CLIP_ACCESS: &CStr = c"OfxImageEffectPropTemporalClipAccess";
    pub const RENDER_THREAD_SAFETY: &CStr = c"OfxImageEffectPluginRenderThreadSafety";
    pub const RENDER_INSTANCE_SAFE: &CStr = c"OfxImageEffectRenderInstanceSafe";
    pub const HOST_FRAME_THREADING: &CStr = c"OfxImageEffectPluginPropHostFrameThreading";
    pub const FIELD_RENDER_TWICE_ALWAYS: &CStr = c"OfxImageEffectPluginPropFieldRenderTwiceAlways";
    pub const SOURCE_CLIP: &CStr = c"Source";
    pub const OUTPUT_CLIP: &CStr = c"Output";
    pub const FRAMES_NEEDED_SOURCE: &CStr = c"OfxImageClipPropFrameRange_Source";
    pub const PREMULTIPLICATION: &CStr = c"OfxImageEffectPropPreMultiplication";
    pub const IMAGE_OPAQUE: &CStr = c"OfxImageOpaque";
    pub const IMAGE_UNPREMULTIPLIED: &CStr = c"OfxImageAlphaUnPremultiplied";
    pub const FRAME_VARYING: &CStr = c"OfxImageEffectFrameVarying";
    pub const FRAME_RATE: &CStr = c"OfxImageEffectPropFrameRate";
    pub const FRAME_RANGE: &CStr = c"OfxImageEffectPropFrameRange";
    pub const RENDER_WINDOW: &CStr = c"OfxImageEffectPropRenderWindow";

    pub const IMAGE_DATA: &CStr = c"OfxImagePropData";
    pub const IMAGE_BOUNDS: &CStr = c"OfxImagePropBounds";
    pub const IMAGE_ROW_BYTES: &CStr = c"OfxImagePropRowBytes";
    pub const PIXEL_DEPTH: &CStr = c"OfxImageEffectPropPixelDepth";
    pub const COMPONENTS: &CStr = c"OfxImageEffectPropComponents";

    pub const PARAM_TYPE_CHOICE: &CStr = c"OfxParamTypeChoice";
    pub const PARAM_TYPE_STRING: &CStr = c"OfxParamTypeString";
    pub const PARAM_TYPE_BOOLEAN: &CStr = c"OfxParamTypeBoolean";
    pub const PARAM_DEFAULT: &CStr = c"OfxParamPropDefault";
    pub const PARAM_CHOICE_OPTION: &CStr = c"OfxParamPropChoiceOption";
    pub const PARAM_HINT: &CStr = c"OfxParamPropHint";
    pub const PARAM_ANIMATES: &CStr = c"OfxParamPropAnimates";
    pub const PARAM_STRING_MODE: &CStr = c"OfxParamPropStringMode";
    pub const PARAM_STRING_IS_FILE_PATH: &CStr = c"OfxParamStringIsFilePath";
    pub const PARAM_STRING_FILE_PATH_EXISTS: &CStr = c"OfxParamPropStringFilePathExists";

    pub const MESSAGE_ERROR: &CStr = c"OfxMessageError";
}
