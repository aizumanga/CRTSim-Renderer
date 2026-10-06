// CC0. Port of J. Kyle Pittman's CRTSim (upstream dbbe9d1).
// Gamma-space UNORM reference pipeline. No NES palette LUT is invented here.
struct Params {
    mvp: mat4x4<f32>,
    size: vec4<f32>,
    signal: vec4<f32>, // sharpness, bleed, artifacts, phase
    persistence: vec4<f32>, // rgb, artifact pattern flipped
    geometry: vec4<f32>, // UV scale xy, overscan, barrel
    mask: vec4<f32>, // repeats xy, brightness, opacity
    lighting: vec4<f32>, // diffuse, specular, power, rim
    surface: vec4<f32>, // dimming, reflection, saturation, unused
    bezel: vec4<f32>,
    light: vec4<f32>,
    camera: vec4<f32>,
    bloom: vec4<f32>, // amount, power, spread, unused
    processing: vec4<f32>, // linear-light surface/bloom path, interlaced, field scanned this tick, screen only
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
    // Flipped, the pattern runs from the bottom up, so the second sample is the row above.
    let v = select(q.uv.y,1.-q.uv.y,p.persistence.w>0.5);
    let auv = vec2(q.uv.x,v) * p.size.xy / vec2(256.,224.);
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

// A point of the glass or the bezel, with what shading it needs.
struct Surface {
    normal: vec3<f32>, color: vec4<f32>, uv: vec2<f32>,
    reflection: f32, camera: vec3<f32>, light: vec3<f32>,
};
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
// The signal on the glass at `uv`, through the mask; `across` and `down` are how uv changes
// from one pixel to the next, which say how small the mask is drawn.
fn crt(uv: vec2<f32>, across: vec2<f32>, down: vec2<f32>, bezel: bool) -> vec3<f32> {
    let scaled=(uv-vec2(0.5))*p.geometry.xy+vec2(0.5);
    let density=p.mask.xy*select(vec2(1.),vec2(1.,0.5),bezel);
    let mask_uv=scaled*density;
    let texels=vec2<f32>(textureDimensions(mask_tex));
    let scale=p.geometry.xy*density*texels;
    let footprint=max(length(across*scale),length(down*scale));
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
fn shade(v: Surface, across: vec2<f32>, down: vec2<f32>, bezel: bool) -> vec4<f32> {
    let n=safe_normalize(v.normal); let cam=safe_normalize(v.camera); let light=safe_normalize(v.light);
    let diffuse=max(dot(n,light),0.);
    let halfvec=safe_normalize(light+cam);
    let spec=pow(max(dot(n,halfvec),0.),p.lighting.z);
    let fres=pow(1.-dot(cam,n),2.)*p.lighting.w;
    var color=crt(v.uv,across,down,bezel);
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
// this sphere: its UVs and normals are functions of where a ray meets it.
const GLASS_CENTRE = vec3(4.,0.,0.);
const GLASS_RADIUS = 4.;
// The mesh's outline and darkened edges, baked across the glass's UVs (crtsim-core's glass
// module): its shade, and the distance to its outline, positive inside and within ±GLASS_EDGE.
@group(0) @binding(12) var glass_map: texture_2d<f32>;
const GLASS_EDGE = 0.015625;
// The bezel, baked from its mesh seen straight on (crtsim-core's bezel module): a depth for
// each point across it, and what the mesh holds there, 16-bit values split over two channels.
@group(0) @binding(9) var bezel_shape: texture_2d<f32>;
@group(0) @binding(10) var bezel_uv: texture_2d<f32>;
@group(0) @binding(11) var bezel_normal: texture_2d<f32>;
const BEZEL_HALF = vec2(1.6533334,1.32);
const BEZEL_NEAR = -0.1;
const BEZEL_FAR = 0.33;
const BEZEL_OPENING = vec2(1.24,0.95);
// Steps across the bezel's depth, then halvings of the step a ray first passes the surface in.
const BEZEL_STEPS = 32;
const BEZEL_HALVINGS = 7;
const NOTHING = 1e9;

fn pair(hi: f32, lo: f32) -> f32 { return (round(hi*255.)*256.+round(lo*255.))/65534.; }
// Where (y, z) falls on the bezel's grid, in texels.
fn bezel_texel(yz: vec2<f32>) -> vec2<f32> {
    let size=vec2<f32>(textureDimensions(bezel_shape));
    return (BEZEL_HALF-yz)/(2.*BEZEL_HALF)*size-vec2(0.5);
}
fn bezel_load(t: texture_2d<f32>, xy: vec2<i32>) -> vec4<f32> {
    let size=vec2<i32>(textureDimensions(t));
    return textureLoad(t,clamp(xy,vec2(0),size-vec2(1)),0);
}
fn bezel_depth_at(xy: vec2<i32>) -> f32 {
    let size=vec2<i32>(textureDimensions(bezel_shape));
    let texel=bezel_load(bezel_shape,xy);
    let empty=any(xy<vec2(0)) || any(xy>=size) || (texel.r>0.999 && texel.g>0.999);
    return select(mix(BEZEL_NEAR,BEZEL_FAR,pair(texel.r,texel.g)),NOTHING,empty);
}
// The bezel's depth at (y, z), blended between texels; nothing where any of them is empty.
fn bezel_depth(yz: vec2<f32>) -> f32 {
    let t=bezel_texel(yz); let xy=vec2<i32>(floor(t)); let f=fract(t);
    let a=bezel_depth_at(xy); let b=bezel_depth_at(xy+vec2(1,0));
    let c=bezel_depth_at(xy+vec2(0,1)); let d=bezel_depth_at(xy+vec2(1,1));
    let blended=mix(mix(a,b,f.x),mix(c,d,f.x),f.y);
    return select(blended,NOTHING,max(max(a,b),max(c,d))>=NOTHING);
}
// An image's two values decoded, blended between texels at texel position g.
fn pairs(t: texture_2d<f32>, g: vec2<f32>) -> vec2<f32> {
    let xy=vec2<i32>(floor(g)); let f=fract(g);
    let decode=array(bezel_load(t,xy),bezel_load(t,xy+vec2(1,0)),bezel_load(t,xy+vec2(0,1)),
        bezel_load(t,xy+vec2(1,1)));
    var v: array<vec2<f32>,4>;
    for (var i=0; i<4; i++) { v[i]=vec2(pair(decode[i].r,decode[i].g),pair(decode[i].b,decode[i].a)); }
    return mix(mix(v[0],v[1],f.x),mix(v[2],v[3],f.x),f.y);
}
fn bezel_pairs(t: texture_2d<f32>, yz: vec2<f32>) -> vec2<f32> { return pairs(t,bezel_texel(yz)); }
// The glass's shade and distance to its outline at uv; texel (i, j) is uv (i, j) / (size - 1).
fn glass_baked(uv: vec2<f32>) -> vec2<f32> {
    let last=vec2<f32>(textureDimensions(glass_map))-vec2(1.);
    let v=pairs(glass_map,clamp(uv,vec2(0.),vec2(1.))*last);
    return vec2(v.x,mix(-GLASS_EDGE,GLASS_EDGE,v.y));
}
fn bezel_bytes(yz: vec2<f32>) -> vec2<f32> {
    let g=bezel_texel(yz); let xy=vec2<i32>(floor(g)); let f=fract(g);
    let a=bezel_load(bezel_shape,xy).ba; let b=bezel_load(bezel_shape,xy+vec2(1,0)).ba;
    let c=bezel_load(bezel_shape,xy+vec2(0,1)).ba; let d=bezel_load(bezel_shape,xy+vec2(1,1)).ba;
    return mix(mix(a,b,f.x),mix(c,d,f.x),f.y);
}
// The point a ray from the camera along `ray` meets at depth x.
fn along(ray: vec3<f32>, x: f32) -> vec3<f32> {
    return p.camera.xyz+ray*((x-p.camera.x)/ray.x);
}
// Where the ray first meets the bezel, NOTHING when it does not: stepping across its depths on
// the depth's high byte, filtered by the sampler, then halving the step it passed the surface
// in on the full depth. It takes no derivatives, so it may stop at the first step inside.
fn bezel_hit(ray: vec3<f32>) -> f32 {
    let step=(BEZEL_FAR-BEZEL_NEAR)/f32(BEZEL_STEPS);
    let size=vec2<f32>(textureDimensions(bezel_shape));
    var before=BEZEL_NEAR-step; var after=NOTHING;
    for (var i=0; i<=BEZEL_STEPS; i++) {
        let x=BEZEL_NEAR+step*f32(i);
        let at=(bezel_texel(along(ray,x).yz)+vec2(0.5))/size;
        let coarse=mix(BEZEL_NEAR,BEZEL_FAR,
            textureSampleLevel(bezel_shape,linear_clamp,at,0.).r*255.*256./65534.);
        if x>=coarse && all(at>=vec2(0.)) && all(at<=vec2(1.)) { after=x; break; }
        before=x;
    }
    var low=before; var high=after;
    for (var i=0; i<BEZEL_HALVINGS; i++) {
        let middle=0.5*(low+high);
        let inside=middle>=bezel_depth(along(ray,middle).yz);
        high=select(high,middle,inside);
        low=select(middle,low,inside);
    }
    let found=after<NOTHING && bezel_depth(along(ray,high).yz)<NOTHING;
    return select(NOTHING,high,found);
}
// Whether a ray can meet the bezel at all: not if it stays within the opening's clear box
// across the bezel's whole depth.
fn may_meet_bezel(ray: vec3<f32>) -> bool {
    let near=abs(along(ray,BEZEL_NEAR).yz); let far=abs(along(ray,BEZEL_FAR).yz);
    return any(max(near,far)>=BEZEL_OPENING);
}
// The glass and its bezel, each pixel showing whichever its ray meets first.
@fragment fn surface(q: Quad) -> @location(0) vec4<f32> {
    let ndc=vec2(q.position.x/p.size.z*2.-1.,1.-q.position.y/p.size.w*2.);
    // The camera looks along +x from 1/tan(fov/2) in front, with z up and -y to the right.
    let spread=-1./p.camera.x;
    let ray=normalize(vec3(1.,-ndc.x*spread*p.size.z/p.size.w,ndc.y*spread));

    let to=p.camera.xyz-GLASS_CENTRE;
    let b=dot(to,ray);
    let reach=b*b-dot(to,to)+GLASS_RADIUS*GLASS_RADIUS;
    let glass_at=p.camera.xyz+ray*(-b-sqrt(max(reach,0.)));
    var glass: Surface;
    glass.uv=vec2((4./3.-glass_at.y)*0.375,(1.-glass_at.z)*0.5);
    let baked=glass_baked(glass.uv);
    glass.normal=(glass_at-GLASS_CENTRE)/GLASS_RADIUS;
    glass.color=vec4(vec3(baked.x),1.);
    glass.reflection=0.;
    glass.camera=p.camera.xyz-glass_at; glass.light=p.light.xyz-glass_at;
    let outside=any(glass.uv<vec2(0.)) || any(glass.uv>vec2(1.)) || baked.y<0.;
    let glass_depth=select(glass_at.x,NOTHING,reach<0. || outside);

    // Only rays that may meet the bezel step across it. Nothing here takes a derivative, so
    // the branch leaves the mask's derivatives below defined.
    var bezel_depth=NOTHING;
    if p.processing.w<0.5 && may_meet_bezel(ray) { bezel_depth=bezel_hit(ray); }
    let bezel_at=along(ray,min(bezel_depth,BEZEL_FAR));
    var bezel: Surface;
    bezel.uv=bezel_pairs(bezel_uv,bezel_at.yz);

    // How each surface's uv changes between pixels, taken here, where every pixel runs, so
    // that each pixel need only shade the surface it shows.
    let glass_across=dpdx(glass.uv); let glass_down=dpdy(glass.uv);
    let bezel_across=dpdx(bezel.uv); let bezel_down=dpdy(bezel.uv);
    if bezel_depth<glass_depth {
        let facing=bezel_pairs(bezel_normal,bezel_at.yz)*2.-vec2(1.);
        bezel.normal=vec3(-sqrt(max(1.-dot(facing,facing),0.)),facing);
        let bytes=bezel_bytes(bezel_at.yz);
        bezel.color=vec4(vec3(bytes.x),1.);
        bezel.reflection=bytes.y;
        bezel.camera=p.camera.xyz-bezel_at; bezel.light=p.light.xyz-bezel_at;
        return shade(bezel,bezel_across,bezel_down,true);
    }
    if glass_depth<NOTHING { return shade(glass,glass_across,glass_down,false); }
    return vec4(0.,0.,0.,1.);
}

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
