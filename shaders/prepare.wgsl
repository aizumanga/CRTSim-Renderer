// CC0. The prepare step on the GPU: alpha composite, source edits, resize, LUT and grade, from
// the decoded input to the logical signal the CRT passes read.
//
// A port of config::prepare, which stays as the reference and the fallback. It keeps that
// code's arithmetic rather than improving on it -- the image crate's separable resize, the
// rounding to 8 bits between stages -- so moving the work changes where it runs and not the
// picture. Channel values are carried in 0-255 steps, as the CPU carries them.
struct Prepare {
    background: vec4<f32>, // rgb in steps, checkerboard
    edit: vec4<f32>, // rotation sin, cos, zoom, nearest
    crop: vec4<f32>, // left, top, width, height, in source pixels
    canvas: vec4<f32>, // pan xy in canvas sizes, signal width, height
    lut_min: vec4<f32>, // domain min, enabled
    lut_max: vec4<f32>, // domain max, size
    grade: vec4<f32>, // hue sin, cos, chroma, enabled
    lut_strength: vec4<f32>, // share of the LUT's colour, unused
};
@group(0) @binding(0) var<uniform> p: Prepare;
@group(0) @binding(1) var input: texture_2d<f32>;
// Per output row or column: first source pixel, tap count, offset into the weights.
@group(0) @binding(2) var spans: texture_2d<u32>;
// Resampling weights computed on the CPU, where they match the image crate's to the bit, laid
// out in rows of a constant power of two, so finding one compiles to a shift and a mask.
@group(0) @binding(3) var weights: texture_2d<f32>;
const WEIGHT_ROW: u32 = 4096u;
@group(0) @binding(4) var lut: texture_3d<f32>;

@vertex fn quad(@builtin(vertex_index) i: u32) -> @builtin(position) vec4<f32> {
    var coords = array<vec2<f32>,3>(vec2(-1.,1.),vec2(-1.,-3.),vec2(3.,1.));
    return vec4(coords[i],0.,1.);
}

// Round half away from zero, as f32::round does; WGSL's round() goes to even.
fn quantize(v: vec3<f32>, top: f32) -> vec3<f32> {
    return floor(clamp(v,vec3(0.),vec3(top))+vec3(0.5));
}
fn steps(c: vec4<f32>) -> vec4<f32> { return floor(c*255.+vec4(0.5)); }

fn weight(i: u32) -> f32 {
    return textureLoad(weights,vec2(i%WEIGHT_ROW,i/WEIGHT_ROW),0).r;
}

// The untransformed composite, which the CPU does in integers: (c*a + bg*(255-a) + 127) / 255.
// Here in floats, which is exact: every term is a whole number below 2^17, and adding half
// before dividing puts the quotient at least 0.5/255 clear of a whole number, far beyond f32
// error, so the floor lands where the integer division does.
fn opaque(xy: vec2<u32>) -> vec3<f32> {
    let c=steps(textureLoad(input,xy,0));
    let n=c.rgb*c.a+p.background.rgb*(255.-c.a)+vec3(127.);
    return floor((n+vec3(0.5))/255.);
}

fn apply_lut(k: vec3<f32>) -> vec3<f32> {
    let last=u32(p.lut_max.w)-1u;
    let xyz=clamp((k/255.-p.lut_min.xyz)/(p.lut_max.xyz-p.lut_min.xyz),vec3(0.),vec3(1.))
        *f32(last);
    let lo=vec3<u32>(floor(xyz));
    let hi=min(lo+vec3(1u),vec3(last));
    let f=xyz-vec3<f32>(lo);
    var out=vec3(0.);
    for (var z=0u; z<2u; z++) {
        for (var y=0u; y<2u; y++) {
            for (var x=0u; x<2u; x++) {
                let at=vec3(select(lo.x,hi.x,x==1u),select(lo.y,hi.y,y==1u),select(lo.z,hi.z,z==1u));
                let w=select(1.-f.x,f.x,x==1u)*select(1.-f.y,f.y,y==1u)*select(1.-f.z,f.z,z==1u);
                out+=textureLoad(lut,at,0).rgb*w;
            }
        }
    }
    let s=p.lut_strength.x;
    return quantize((out*s+(k/255.)*(1.-s))*255.,255.);
}

// Optional YIQ hue rotation and chroma scale.
fn apply_grade(k: vec3<f32>) -> vec3<f32> {
    let c=k/255.;
    let y=0.299*c.r+0.587*c.g+0.114*c.b;
    let i=0.596*c.r-0.274*c.g-0.322*c.b;
    let q=0.211*c.r-0.523*c.g+0.312*c.b;
    let ii=(i*p.grade.y-q*p.grade.x)*p.grade.z;
    let qq=(i*p.grade.x+q*p.grade.y)*p.grade.z;
    let rgb=vec3(y+0.956*ii+0.621*qq,y-0.272*ii-0.647*qq,y-1.106*ii+1.703*qq);
    return quantize(rgb*255.,255.);
}

// Every route ends here, with a colour already rounded to whole steps.
fn finish(k: vec3<f32>) -> vec4<f32> {
    var c=k;
    if p.lut_min.w>0.5 { c=apply_lut(c); }
    if p.grade.w>0.5 { c=apply_grade(c); }
    return vec4(c/255.,1.);
}

// The image crate resizes rows first into unclamped floats, then columns with a clamp and a
// round; the two passes keep that order and that intermediate.
@fragment fn resample_rows(@builtin(position) at: vec4<f32>) -> @location(0) vec4<f32> {
    let xy=vec2<u32>(at.xy);
    let span=textureLoad(spans,vec2(xy.y,0u),0);
    var total=vec3(0.);
    for (var i=0u; i<span.y; i++) {
        total+=opaque(vec2(xy.x,span.x+i))*weight(span.z+i);
    }
    return vec4(total,255.);
}
@fragment fn resample_columns(@builtin(position) at: vec4<f32>) -> @location(0) vec4<f32> {
    let xy=vec2<u32>(at.xy);
    let span=textureLoad(spans,vec2(xy.x,0u),0);
    var total=vec3(0.);
    for (var i=0u; i<span.y; i++) {
        total+=textureLoad(input,vec2(span.x+i,xy.y),0).rgb*weight(span.z+i);
    }
    return finish(quantize(total,255.));
}

// Source edits: crop, rotation, zoom and pan, sampled nearest or bilinear with the alpha
// composite applied per sample, as workflow::SourceEdit::prepare does it.
fn background(xy: vec2<u32>) -> vec3<f32> {
    if p.background.w>0.5 { return vec3(select(128.,192.,(xy.x/16u+xy.y/16u)%2u==0u)); }
    return p.background.rgb;
}
fn edit_sample(ix: i32, iy: i32, bg: vec3<f32>) -> vec3<f32> {
    let fx=f32(ix)+0.5;
    let fy=f32(iy)+0.5;
    let size=vec2<i32>(textureDimensions(input));
    if fx<p.crop.x || fy<p.crop.y || fx>=p.crop.x+p.crop.z || fy>=p.crop.y+p.crop.w
        || ix<0 || iy<0 || ix>=size.x || iy>=size.y {
        return bg;
    }
    let c=steps(textureLoad(input,vec2(ix,iy),0));
    let a=c.a/255.;
    return c.rgb*a+bg*(1.-a);
}
@fragment fn edit(@builtin(position) at: vec4<f32>) -> @location(0) vec4<f32> {
    let xy=vec2<u32>(at.xy);
    let px=((f32(xy.x)+0.5)/p.canvas.z-0.5-p.canvas.x)*p.crop.z/p.edit.z;
    let py=((f32(xy.y)+0.5)/p.canvas.w-0.5-p.canvas.y)*p.crop.w/p.edit.z;
    let sx=p.edit.y*px+p.edit.x*py+p.crop.z*0.5;
    let sy=-p.edit.x*px+p.edit.y*py+p.crop.w*0.5;
    let bg=background(xy);
    var color: vec3<f32>;
    if p.edit.w>0.5 {
        color=edit_sample(i32(floor(p.crop.x+sx)),i32(floor(p.crop.y+sy)),bg);
    } else {
        let fx=p.crop.x+sx-0.5;
        let fy=p.crop.y+sy-0.5;
        let ix=i32(floor(fx));
        let iy=i32(floor(fy));
        let dx=fx-floor(fx);
        let dy=fy-floor(fy);
        let a=edit_sample(ix,iy,bg);
        let b=edit_sample(ix+1,iy,bg);
        let c=edit_sample(ix,iy+1,bg);
        let d=edit_sample(ix+1,iy+1,bg);
        color=(a*(1.-dx)+b*dx)*(1.-dy)+(c*(1.-dx)+d*dx)*dy;
    }
    return finish(quantize(color,255.));
}
