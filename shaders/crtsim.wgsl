// CC0. Port of J. Kyle Pittman's CRTSim (upstream dbbe9d1).
// Gamma-space UNORM reference pipeline. No NES palette LUT is invented here.
struct Params {
    mvp: mat4x4<f32>,
    size: vec4<f32>,
    signal: vec4<f32>, // sharpness, bleed, artifacts, phase
    persistence: vec4<f32>,
    geometry: vec4<f32>, // UV scale xy, overscan, barrel
    mask: vec4<f32>, // repeats xy, brightness, opacity
    lighting: vec4<f32>, // diffuse, specular, power, rim
    surface: vec4<f32>, // dimming, reflection, saturation, unused
    bezel: vec4<f32>,
    light: vec4<f32>,
    camera: vec4<f32>,
    bloom: vec4<f32>, // amount, power, spread, unused
    processing: vec4<f32>, // linear-light surface/bloom path, interlaced, field scanned this tick
};
@group(0) @binding(0) var<uniform> p: Params;
@group(0) @binding(1) var source: texture_2d<f32>;
@group(0) @binding(2) var previous: texture_2d<f32>;
@group(0) @binding(3) var artifacts: texture_2d<f32>;
@group(0) @binding(4) var mask_tex: texture_2d<f32>;
@group(0) @binding(5) var point_clamp: sampler;
@group(0) @binding(6) var linear_clamp: sampler;
@group(0) @binding(7) var point_repeat: sampler;
@group(0) @binding(8) var linear_repeat: sampler;

struct Quad { @builtin(position) position: vec4<f32>, @location(0) uv: vec2<f32> };
@vertex fn quad(@builtin(vertex_index) i: u32) -> Quad {
    var coords = array<vec2<f32>,3>(vec2(-1.,1.),vec2(-1.,-3.),vec2(3.,1.));
    var o: Quad; o.position = vec4(coords[i],0.,1.);
    o.uv = coords[i] * vec2(0.5,-0.5) + vec2(0.5); return o;
}
fn luma(c: vec3<f32>) -> f32 { return dot(c,vec3(0.299,0.587,0.114)); }
fn srgb_decode(c: vec3<f32>) -> vec3<f32> {
    return select(pow((max(c,vec3(0.))+vec3(0.055))/1.055,vec3(2.4)),c/12.92,c<=vec3(0.04045));
}
fn srgb_encode(c: vec3<f32>) -> vec3<f32> {
    return select(1.055*pow(max(c,vec3(0.)),vec3(1./2.4))-vec3(0.055),c*12.92,c<=vec3(0.0031308));
}
fn display_luma(c: vec3<f32>) -> f32 {
    return select(luma(c),dot(c,vec3(0.2126,0.7152,0.0722)),p.processing.x>0.5);
}
@fragment fn composite(q: Quad) -> @location(0) vec4<f32> {
    let dx = vec2(1./p.size.x,0.);
    // Original artifact tile covers 256x224 logical pixels; do not stretch at new sizes.
    let auv = q.uv * p.size.xy / vec2(256.,224.);
    let a = mix(textureSampleLevel(artifacts,point_repeat,auv,0.),
        textureSampleLevel(artifacts,point_repeat,auv+vec2(0.,1./224.),0.),p.signal.w).rgb;
    let left = textureSampleLevel(source,point_clamp,q.uv-dx,0.).rgb;
    let right = textureSampleLevel(source,point_clamp,q.uv+dx,0.).rgb;
    let cur = textureSampleLevel(source,point_clamp,q.uv,0.).rgb;
    var color = clamp(cur + (left+right-2.*cur)*a*p.signal.z,vec3(0.),vec3(1.));
    let brt = luma(color);
    var offset = 0.;
    var weights = array<f32,3>(1.,-0.3162277,0.1);
    for (var i=0u; i<3u; i++) {
        let step = dx*f32(i+1u);
        let lb = luma(textureSampleLevel(source,point_clamp,q.uv-step,0.).rgb);
        let rb = luma(textureSampleLevel(source,point_clamp,q.uv+step,0.).rgb);
        offset += (2.*brt-lb-rb)*weights[i];
    }
    color = clamp(color+offset*p.signal.x*mix(vec3(1.),a,p.signal.z),vec3(0.),vec3(1.));
    // Interlaced: the beam skips the other field's rows this tick; they only decay below.
    if p.processing.y>0.5 && (u32(q.position.y)%2u)!=u32(p.processing.z) { color=vec3(0.); }
    let pl = textureSampleLevel(previous,point_clamp,q.uv-dx,0.).rgb;
    let pr = textureSampleLevel(previous,point_clamp,q.uv+dx,0.).rgb;
    let pc = textureSampleLevel(previous,point_clamp,q.uv,0.).rgb;
    color = max(color,p.persistence.rgb*(pc+(pl+pr)*p.signal.y)/(1.+2.*p.signal.y));
    return vec4(clamp(color,vec3(0.),vec3(1.)),1.);
}

struct MeshInput {
    @location(0) position: vec3<f32>, @location(1) normal: vec3<f32>,
    @location(2) color: vec4<f32>, @location(3) uv: vec2<f32>, @location(4) reflection: f32,
};
struct Surface {
    @builtin(position) position: vec4<f32>, @location(0) normal: vec3<f32>,
    @location(1) color: vec4<f32>, @location(2) uv: vec2<f32>,
    @location(3) reflection: f32, @location(4) camera: vec3<f32>, @location(5) light: vec3<f32>,
};
@vertex fn mesh(v: MeshInput) -> Surface {
    var o: Surface; o.position=p.mvp*vec4(v.position,1.); o.normal=v.normal;
    o.color=v.color; o.uv=v.uv; o.reflection=v.reflection;
    o.camera=p.camera.xyz-v.position; o.light=p.light.xyz-v.position; return o;
}
// Reproduce D3D9's black border including bilinear filtering at the boundary.
fn border_load(xy: vec2<i32>) -> vec3<f32> {
    let sz=vec2<i32>(textureDimensions(source));
    if (any(xy<vec2(0)) || any(xy>=sz)) { return vec3(0.); }
    let c=textureLoad(source,xy,0).rgb;
    return select(c,srgb_decode(c),p.processing.x>0.5);
}
fn black_border(uv: vec2<f32>) -> vec3<f32> {
    let pixel=uv*vec2<f32>(textureDimensions(source))-vec2(0.5);
    let xy=vec2<i32>(floor(pixel)); let f=fract(pixel);
    return mix(mix(border_load(xy),border_load(xy+vec2(1,0)),f.x),
        mix(border_load(xy+vec2(0,1)),border_load(xy+vec2(1,1)),f.x),f.y);
}
fn crt(uv: vec2<f32>, bezel: bool) -> vec3<f32> {
    let scaled=(uv-vec2(0.5))*p.geometry.xy+vec2(0.5);
    let density=p.mask.xy*select(vec2(1.),vec2(1.,0.5),bezel);
    let mask_uv=scaled*density;
    let texels=vec2<f32>(textureDimensions(mask_tex));
    let footprint=max(length(dpdx(mask_uv)*texels),length(dpdy(mask_uv)*texels));
    let lod=select(0.,max(log2(max(footprint,0.000001)),0.),p.surface.w>0.5);
    var grid=textureSampleLevel(mask_tex,linear_repeat,mask_uv,lod).rgb;
    grid=mix(vec3(1.),grid+vec3(p.mask.z),p.mask.w);
    let over=select(1./p.geometry.z,p.geometry.z,bezel);
    var pos=(scaled-vec2(0.5))*over;
    pos=pos+pos*p.geometry.w*dot(pos,pos)+vec2(0.5);
    let emissive=black_border(pos)*grid;
    return mix(vec3(display_luma(emissive)),emissive,p.surface.z);
}
fn safe_normalize(v: vec3<f32>) -> vec3<f32> {
    if dot(v,v)<0.000000000001 { return vec3(0.); }
    return normalize(v);
}
fn shade(v: Surface, bezel: bool) -> vec4<f32> {
    let n=safe_normalize(v.normal); let cam=safe_normalize(v.camera); let light=safe_normalize(v.light);
    let diffuse=max(dot(n,light),0.);
    let halfvec=safe_normalize(light+cam);
    let spec=pow(max(dot(n,halfvec),0.),p.lighting.z);
    let fres=pow(1.-dot(cam,n),2.)*p.lighting.w;
    var color=crt(v.uv,bezel);
    if bezel {
        let hemi=(dot(n,vec3(0.,0.,1.))*0.5+0.5)*0.4+0.3;
        let bezel_color=select(p.bezel.rgb,srgb_decode(p.bezel.rgb),p.processing.x>0.5);
        color=bezel_color*(diffuse+hemi)*p.lighting.x + vec3(0.25)*spec*p.lighting.y
            +color*v.reflection*p.surface.y+vec3(0.15)*fres;
    } else {
        color+=vec3(0.175,0.15,0.2)*diffuse*p.lighting.x
            +vec3(0.25)*spec*p.lighting.y+vec3(0.45,0.4,0.5)*fres;
    }
    return vec4(color*mix(vec3(1.),v.color.rgb,p.surface.x),1.);
}
// The screen's glass, ray-traced rather than drawn as the original's mesh, which was a cap of
// this sphere: its UVs, normals, rounded outline and darkened edges are all functions of where
// a ray meets it. A host that can only draw full-screen passes draws it the same way.
const GLASS_CENTRE = vec3(4.,0.,0.);
const GLASS_RADIUS = 4.;
// The outline is a superellipse in UV space; its edge shading follows the same curves inward.
const GLASS_CORNER = 11.54;
struct Glass { @builtin(frag_depth) depth: f32, @location(0) color: vec4<f32> };
@fragment fn glass(q: Quad) -> Glass {
    let ndc=vec2(q.position.x/p.size.z*2.-1.,1.-q.position.y/p.size.w*2.);
    // The camera looks along +x from 1/tan(fov/2) in front, with z up and -y to the right.
    let spread=-1./p.camera.x;
    let ray=normalize(vec3(1.,-ndc.x*spread*p.size.z/p.size.w,ndc.y*spread));
    let to=p.camera.xyz-GLASS_CENTRE;
    let b=dot(to,ray);
    let reach=b*b-dot(to,to)+GLASS_RADIUS*GLASS_RADIUS;
    let hit=p.camera.xyz+ray*(-b-sqrt(max(reach,0.)));
    var v: Surface;
    v.uv=vec2((4./3.-hit.y)*0.375,(1.-hit.z)*0.5);
    let edge=abs(v.uv*2.-vec2(1.));
    let radius=pow(pow(edge.x,GLASS_CORNER)+pow(edge.y,GLASS_CORNER),1./GLASS_CORNER);
    v.normal=(hit-GLASS_CENTRE)/GLASS_RADIUS;
    v.color=vec4(vec3(1.-0.5*pow(radius,6.)),1.);
    v.reflection=0.;
    v.camera=p.camera.xyz-hit; v.light=p.light.xyz-hit;
    var o: Glass;
    o.color=shade(v,false);
    // A ray that misses the glass lies at the far plane, which the depth test turns away, so
    // everything above stays in uniform control flow for the mask's derivatives.
    let clip=p.mvp*vec4(hit,1.);
    o.depth=select(clip.z/clip.w,1.,reach<0. || radius>1.);
    return o;
}
@fragment fn bezel(v: Surface) -> @location(0) vec4<f32> { return shade(v,true); }

fn blur(uv: vec2<f32>, swap: bool) -> vec4<f32> {
    var offsets=array<vec2<f32>,7>(vec2(0.,0.),vec2(0.,1.),vec2(0.,-1.),
        vec2(-0.866025,0.5),vec2(-0.866025,-0.5),vec2(0.866025,0.5),vec2(0.866025,-0.5));
    let scale=vec2(p.size.w/p.size.z,1.)*p.bloom.z;
    var total=vec3(0.);
    for (var i=0u;i<7u;i++) {
        let off=select(offsets[i],offsets[i].yx,swap);
        total+=textureSampleLevel(source,linear_clamp,uv+off*scale,0.).rgb;
    }
    return vec4(total/7.,1.);
}
@fragment fn downsample(q: Quad) -> @location(0) vec4<f32> { return blur(q.uv,false); }
@fragment fn upsample(q: Quad) -> @location(0) vec4<f32> { return blur(q.uv,true); }
@fragment fn present(q: Quad) -> @location(0) vec4<f32> {
    let base=textureSampleLevel(source,linear_clamp,q.uv,0.).rgb;
    let blurred=textureSampleLevel(previous,linear_clamp,q.uv,0.).rgb;
    let lum=display_luma(blurred);
    // Upstream left a TODO here: explicitly avoid division by zero on black.
    let colored=blurred/max(lum,0.000001)*pow(max(lum,0.),p.bloom.y);
    let result=base+colored*p.bloom.x;
    return vec4(select(result,srgb_encode(result),p.processing.x>0.5),1.);
}
