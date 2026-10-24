// CC0. Port of J. Kyle Pittman's CRTSim (upstream dbbe9d1).
// Gamma-space UNORM reference pipeline. No NES palette LUT is invented here.
struct Params {
    size: vec4<f32>,
    signal: vec4<f32>, // sharpness, bleed, artifacts, phase
    persistence: vec4<f32>, // rgb, artifact pattern flipped
    geometry: vec4<f32>, // UV scale xy, overscan, barrel
    mask: vec4<f32>, // repeats xy, brightness, opacity
    lighting: vec4<f32>, // diffuse, specular, power, rim
    surface: vec4<f32>, // dimming, reflection, saturation, mask filtered when drawn smaller
    bezel: vec4<f32>, // colour, reflection as on the screen
    light: vec4<f32>, // position, the glass's stretch across from the original's
    camera: vec4<f32>, // position, the tube's width over its height
    bloom: vec4<f32>, // amount, power, spread, the tube's widening either side
    processing: vec4<f32>, // linear-light surface/bloom path, interlaced, field scanned this tick, screen only
    backdrop: vec4<f32>, // colour where neither the glass nor the bezel is, the view's widening
    post: vec4<f32>, // bloom read through the picture's mipmaps, glow floor, faceted glass, unused
    finish: vec4<f32>, // brightness's power, FXAA level, tate's quarter turns, second blur's spread
    picture: vec4<f32>, // pixel aspect, TV static, its cycle in seconds, its clock in 60ths
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
// The level 8-bit history holds each channel's glow at once lit, when the glow floor is on: the
// brightest level its persistence rounds back to itself, none where a frame of no time keeps
// all of it.
fn glow_floor() -> vec3<f32> {
    let kept=p.persistence.rgb;
    let level=min(floor(0.5/max(vec3(1.)-kept,vec3(0.000001))),vec3(255.))/255.;
    return select(vec3(0.),level,(kept<vec3(1.)) & vec3(p.post.y>0.5));
}
fn display_luma(c: vec3<f32>) -> f32 {
    return select(luma(c),dot(c,vec3(0.2126,0.7152,0.0722)),p.processing.x>0.5);
}
// TV static, this renderer's own: white noise drawn anew each sixtieth of a second, squared, rolling under a
// darker band, over the picture wavering across and slipping down as the static thickens.
fn static_hash(x: u32, y: u32, tick: u32) -> f32 {
    var h=x*0x8da6b343u ^ y*0xd8163841u ^ tick*0xcb1ab31fu;
    h=(h ^ (h >> 16u))*0x7feb352du;
    h=(h ^ (h >> 15u))*0x846ca68bu;
    return f32(h ^ (h >> 16u))/4294967295.;
}
// How strong the static is this tick: its amount, or with a cycle, the amount over a channel
// change's course: static, fading to the picture, the picture, fading back.
fn static_strength() -> f32 {
    let cycle=p.picture.z;
    if cycle<=0. { return p.picture.y; }
    let u=fract(p.picture.w/60./cycle);
    let fade=clamp((0.5-u)/0.1333333,0.,1.)+clamp((u-0.8666667)/0.1333333,0.,1.);
    return p.picture.y*clamp(select(fade,1.,u<0.3666667),0.,1.);
}
fn picture_at(uv: vec2<f32>, strength: f32) -> vec3<f32> {
    if strength<=0. { return textureSampleLevel(source,point_clamp,uv,0.).rgb; }
    let t=p.picture.w/60.;
    let row=uv.y;
    let waver=(sin(6.2831853*(row*2.3+t*1.7))+0.5*sin(6.2831853*(row*5.9-t*3.1))
        +0.25*sin(6.2831853*(row*13.7+t*7.3)))*0.012*strength;
    let slip=strength*strength*0.03;
    let shown=textureSampleLevel(source,point_clamp,vec2(uv.x+waver,uv.y-slip),0.).rgb;
    let texel=vec2<u32>(clamp(uv*p.size.xy,vec2(0.),p.size.xy-vec2(1.)));
    let noise=static_hash(texel.x,texel.y,u32(p.picture.w));
    let band=(fract(row-t*0.37)-0.5)/0.08;
    let dark=1.-0.55*exp(-band*band);
    let lit=noise*dark;
    let snow=lit*lit*1.4;
    return mix(shown,vec3(min(snow,1.)),strength);
}
@fragment fn composite(q: Quad) -> @location(0) vec4<f32> {
    let strength = static_strength();
    let dx = vec2(1./p.size.x,0.);
    // Original artifact tile covers 256x224 logical pixels; do not stretch at new sizes.
    // Flipped, the pattern runs from the bottom up, so the second sample is the row above.
    let v = select(q.uv.y,1.-q.uv.y,p.persistence.w>0.5);
    let auv = vec2(q.uv.x,v) * p.size.xy / vec2(256.,224.);
    let a = mix(textureSampleLevel(artifacts,point_repeat,auv,0.),
        textureSampleLevel(artifacts,point_repeat,auv+vec2(0.,1./224.),0.),p.signal.w).rgb;
    let left = picture_at(q.uv-dx,strength);
    let right = picture_at(q.uv+dx,strength);
    let cur = picture_at(q.uv,strength);
    var color = clamp(cur + (left+right-2.*cur)*a*p.signal.z,vec3(0.),vec3(1.));
    let brt = luma(color);
    var offset = 0.;
    var weights = array<f32,3>(1.,-0.3162277,0.1);
    for (var i=0u; i<3u; i++) {
        let step = dx*f32(i+1u);
        let lb = luma(picture_at(q.uv-step,strength));
        let rb = luma(picture_at(q.uv+step,strength));
        offset += (2.*brt-lb-rb)*weights[i];
    }
    color = clamp(color+offset*p.signal.x*mix(vec3(1.),a,p.signal.z),vec3(0.),vec3(1.));
    // Interlaced: the beam skips the other field's rows this tick; they only decay below.
    if p.processing.y>0.5 && (u32(q.position.y)%2u)!=u32(p.processing.z) { color=vec3(0.); }
    let held = glow_floor();
    let pl = max(textureSampleLevel(previous,point_clamp,q.uv-dx,0.).rgb,held);
    let pr = max(textureSampleLevel(previous,point_clamp,q.uv+dx,0.).rgb,held);
    let pc = max(textureSampleLevel(previous,point_clamp,q.uv,0.).rgb,held);
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
    // The public source reflects half the mask's rows in the bezel, and the overscan
    // unreciprocated; Super Win the Game reflects them as the screen shows them.
    let as_screen=!bezel || p.bezel.w>0.5;
    let density=p.mask.xy*select(vec2(1.,0.5),vec2(1.),as_screen);
    let mask_uv=scaled*density;
    let texels=vec2<f32>(textureDimensions(mask_tex));
    let scale=p.geometry.xy*density*texels;
    let footprint=max(length(across*scale),length(down*scale));
    let lod=select(0.,max(log2(max(footprint,0.000001)),0.),p.surface.w>0.5);
    var grid=textureSampleLevel(mask_tex,linear_repeat,mask_uv,lod).rgb;
    grid=mix(vec3(1.),grid+vec3(p.mask.z),p.mask.w);
    let over=select(p.geometry.z,1./p.geometry.z,as_screen);
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
    // Squared by multiplying, as the original does, which pow leaves undefined below 0.
    let grazing=1.-dot(cam,n);
    let fres=grazing*grazing*p.lighting.w;
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
// The mesh's outline, darkened edges and facets, baked across the glass's UVs (crtsim-core's
// glass module), two maps one above the other: its shade, and the distance to its outline,
// positive inside and within ±GLASS_EDGE; then how far behind the sphere its flat triangles lie,
// up to GLASS_FACET.
@group(0) @binding(12) var glass_map: texture_2d<f32>;
const GLASS_EDGE = 0.015625;
const GLASS_FACET = 0.00048828125;
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

// A tube other than the original's 4:3 is as much wider, or narrower, either side: its glass
// the original's stretched across, its bezel's corners and sides moved out, or in, and its top
// and bottom running between them, as the original's middle.
// Both are worked out with the settings, so the original's are exactly none and 1.
fn widened() -> f32 { return p.bloom.w; }
fn stretched() -> f32 { return p.light.w; }
// Where y across a tube this wide falls across the original's.
fn original_y(y: f32) -> f32 { return sign(y)*max(abs(y)-widened(),0.); }

fn pair(hi: f32, lo: f32) -> f32 { return (round(hi*255.)*256.+round(lo*255.))/65534.; }
// Where (y, z) falls on the bezel's grid, in texels.
fn bezel_texel(yz: vec2<f32>) -> vec2<f32> {
    let size=vec2<f32>(textureDimensions(bezel_shape));
    return (BEZEL_HALF-vec2(original_y(yz.x),yz.y))/(2.*BEZEL_HALF)*size-vec2(0.5);
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
    let last=vec2<f32>(textureDimensions(glass_map))*vec2(1.,0.5)-vec2(1.);
    let v=pairs(glass_map,clamp(uv,vec2(0.),vec2(1.))*last);
    return vec2(v.x,mix(-GLASS_EDGE,GLASS_EDGE,v.y));
}
// How far behind the sphere the mesh's facet lies at uv, along the view.
fn glass_facet(uv: vec2<f32>) -> f32 {
    let half=vec2<f32>(textureDimensions(glass_map))*vec2(1.,0.5);
    return pairs(glass_map,clamp(uv,vec2(0.),vec2(1.))*(half-vec2(1.))+vec2(0.,half.y)).x
        *GLASS_FACET;
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
// in on the full depth. At the bezel's outer rim the filtered byte blends in the empty texels
// past it, so a step can seem inside where the full depth is still empty; the march goes on
// from there. It takes no derivatives, so it may stop at the first step inside.
fn bezel_hit(ray: vec3<f32>) -> f32 {
    let step=(BEZEL_FAR-BEZEL_NEAR)/f32(BEZEL_STEPS);
    let size=vec2<f32>(textureDimensions(bezel_shape));
    var before=BEZEL_NEAR-step;
    for (var i=0; i<=BEZEL_STEPS; i++) {
        let x=BEZEL_NEAR+step*f32(i);
        let at=(bezel_texel(along(ray,x).yz)+vec2(0.5))/size;
        let coarse=mix(BEZEL_NEAR,BEZEL_FAR,
            textureSampleLevel(bezel_shape,linear_clamp,at,0.).r*255.*256./65534.);
        if x>=coarse && all(at>=vec2(0.)) && all(at<=vec2(1.)) {
            var low=before; var high=x;
            for (var j=0; j<BEZEL_HALVINGS; j++) {
                let middle=0.5*(low+high);
                let inside=middle>=bezel_depth(along(ray,middle).yz);
                high=select(high,middle,inside);
                low=select(middle,low,inside);
            }
            if bezel_depth(along(ray,high).yz)<NOTHING { return high; }
        }
        before=x;
    }
    return NOTHING;
}
// Whether a ray can meet the bezel at all: not if it stays within the opening's clear box
// across the bezel's whole depth.
fn may_meet_bezel(ray: vec3<f32>) -> bool {
    let near=abs(along(ray,BEZEL_NEAR).yz); let far=abs(along(ray,BEZEL_FAR).yz);
    return any(max(near,far)>=BEZEL_OPENING+vec2(widened(),0.));
}
// The glass and its bezel, each pixel showing whichever its ray meets first.
@fragment fn surface(q: Quad) -> @location(0) vec4<f32> {
    let ndc=vec2(q.position.x/p.size.z*2.-1.,1.-q.position.y/p.size.w*2.);
    // The camera looks along +x from 1/tan(fov/2) in front, with z up and -y to the right. A
    // tube wider than the picture fits across it, as the game fits its 4:3 tube across tate's
    // portrait picture.
    let spread=-1./p.camera.x*p.backdrop.w;
    // The ray where the glass is the original's sphere, across `stretched` times narrower;
    // along any ray, the bezel steps by depth alone, so it takes the ray stretched back.
    let s=stretched();
    let traced=normalize(vec3(1.,-ndc.x*spread*p.size.z/p.size.w/s,ndc.y*spread));
    let ray=vec3(traced.x,traced.y*s,traced.z);

    let to=p.camera.xyz-GLASS_CENTRE;
    let b=dot(to,traced);
    let reach=b*b-dot(to,to)+GLASS_RADIUS*GLASS_RADIUS;
    var sphere_at=p.camera.xyz+traced*(-b-sqrt(max(reach,0.)));
    if p.post.z>0.5 {
        // On to the mesh's flat triangle there, as far behind as the facet lies: one step of
        // Newton's along the ray, which lands on it within a ten-thousandth of a signal pixel
        // on average.
        let facet=glass_facet(vec2((4./3.-sphere_at.y)*0.375,(1.-sphere_at.z)*0.5));
        let from_centre=sphere_at-GLASS_CENTRE;
        sphere_at+=traced*(facet*from_centre.x/min(dot(traced,from_centre),-0.000001));
    }
    let glass_at=vec3(sphere_at.x,sphere_at.y*s,sphere_at.z);
    var glass: Surface;
    // The picture spans the whole glass; its outline and shading keep the original's corners.
    glass.uv=vec2((4./3.-sphere_at.y)*0.375,(1.-sphere_at.z)*0.5);
    let baked=glass_baked(vec2((4./3.-original_y(glass_at.y))*0.375,glass.uv.y));
    glass.normal=(sphere_at-GLASS_CENTRE)/GLASS_RADIUS*vec3(1.,1./s,1.);
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
    if widened()!=0. {
        // The point of the picture the bezel reflects, moved with it: its UV is the screen's
        // across the original's glass.
        let a=p.camera.w;
        let screen_y=4./3.-bezel.uv.x*8./3.+bezel_at.y-original_y(bezel_at.y);
        bezel.uv.x=(a-screen_y)/(2.*a);
    }

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
    return vec4(select(p.backdrop.rgb,srgb_decode(p.backdrop.rgb),p.processing.x>0.5),1.);
}

fn blur(uv: vec2<f32>, swap: bool, lod: f32) -> vec4<f32> {
    var offsets=array<vec2<f32>,7>(vec2(0.,0.),vec2(0.,1.),vec2(0.,-1.),
        vec2(-0.866025,0.5),vec2(-0.866025,-0.5),vec2(0.866025,0.5),vec2(0.866025,-0.5));
    let scale=vec2(p.size.w/p.size.z,1.)*select(p.bloom.z,p.finish.w,swap);
    var total=vec3(0.);
    for (var i=0u;i<7u;i++) {
        let off=select(offsets[i],offsets[i].yx,swap);
        total+=textureSampleLevel(source,linear_clamp,uv+off*scale,lod).rgb;
    }
    return vec4(total/7.,1.);
}
// The bloom's first blur, into a sixteenth of the picture each way, rounded down. Super Win the
// Game reads the picture through its mipmaps, at the level GL picks for a target that size:
// level 4, each tap the average of 16x16 of its pixels, and towards 5 where a side does not
// divide by 16. The public source reads the picture itself.
@fragment fn downsample(q: Quad) -> @location(0) vec4<f32> {
    let ratio=p.size.zw/max(floor(p.size.zw/16.),vec2(1.));
    return blur(q.uv,false,select(0.,log2(max(max(ratio.x,ratio.y),1.)),p.post.x>0.5));
}
@fragment fn upsample(q: Quad) -> @location(0) vec4<f32> { return blur(q.uv,true,0.); }
// A mip level of the picture from the one above, as GL's glGenerateMipmap draws it: filtered
// at each texel's centre, the average of the 2x2 texels above it, or blended between them
// where the level above has an odd size.
@fragment fn halve(q: Quad) -> @location(0) vec4<f32> {
    return vec4(textureSampleLevel(source,linear_clamp,q.uv,0.).rgb,1.);
}
// A colour with its luma raised to `power`, its hue kept, as the game's ColorPow does; black
// stays black.
fn color_pow(c: vec3<f32>, power: f32) -> vec3<f32> {
    let lum=luma(c);
    return c/max(lum,0.000001)*pow(max(lum,0.),power);
}
// Where the turned picture's pixel at uv comes from: Super Win the Game's tate turns the
// finished picture about its middle in quarter turns counter-clockwise.
fn turned(uv: vec2<f32>) -> vec2<f32> {
    let v=uv-vec2(0.5);
    switch u32(p.finish.z) {
        case 1u: { return vec2(-v.y,v.x)+vec2(0.5); }
        case 2u: { return vec2(-v.x,-v.y)+vec2(0.5); }
        case 3u: { return vec2(v.y,-v.x)+vec2(0.5); }
        default: { return uv; }
    }
}
fn surface_at(uv: vec2<f32>) -> vec3<f32> { return textureSampleLevel(source,linear_clamp,uv,0.).rgb; }
// FXAA's luma, from red and green as the game's FXAA weighs them, in gamma space.
fn fxaa_luma(c: vec3<f32>) -> f32 {
    let g=select(c,srgb_encode(c),p.processing.x>0.5);
    return g.g*(0.587/0.299)+g.r;
}
// The surface at a point of GL's texture coordinates, which run up the picture.
fn surface_gl(gl: vec2<f32>) -> vec3<f32> { return surface_at(vec2(gl.x,1.-gl.y)); }
// Super Win the Game's FXAA: Timothy Lottes' FXAA 3 quality, as the game sets its four levels.
// A pixel on an edge, found by the luma range across its neighbours, is filtered along the edge
// by how far it lies from the edge's two ends, found by stepping along it, and blended towards
// its neighbours' average where the edge is finer than a pixel. It runs in GL's coordinates, as
// the game does, so where two ways tie it takes the game's.
fn fxaa(screen_uv: vec2<f32>, level: u32) -> vec3<f32> {
    let uv=vec2(screen_uv.x,1.-screen_uv.y);
    let px=1./vec2<f32>(textureDimensions(source));
    var edge=0.25; var least=1./12.; var steps=4u;
    if level==2u { edge=1./6.; least=1./14.; steps=8u; }
    if level==3u { edge=0.125; least=1./16.; steps=16u; }
    if level==4u { edge=1./16.; least=1./32.; steps=32u; }
    let centre=surface_gl(uv);
    let m=fxaa_luma(centre);
    var n=fxaa_luma(surface_gl(uv+vec2(0.,-px.y)));
    let w=fxaa_luma(surface_gl(uv+vec2(-px.x,0.)));
    let e=fxaa_luma(surface_gl(uv+vec2(px.x,0.)));
    var s=fxaa_luma(surface_gl(uv+vec2(0.,px.y)));
    let lowest=min(m,min(min(n,w),min(s,e)));
    let highest=max(m,max(max(n,w),max(s,e)));
    let range=highest-lowest;
    if range<max(least,highest*edge) { return centre; }
    let nw=fxaa_luma(surface_gl(uv+vec2(-px.x,-px.y)));
    let ne=fxaa_luma(surface_gl(uv+vec2(px.x,-px.y)));
    let sw=fxaa_luma(surface_gl(uv+vec2(-px.x,px.y)));
    let se=fxaa_luma(surface_gl(uv+vec2(px.x,px.y)));
    let average=(n+w+e+s)*0.25;
    let subpixel=min(0.75,clamp(abs(average-m)/range-0.25,0.,1.)/0.75);
    let across=abs(nw-2.*n+ne)+2.*abs(w-2.*m+e)+abs(sw-2.*s+se);
    let down=abs(nw-2.*w+sw)+2.*abs(n-2.*m+s)+abs(ne-2.*e+se);
    // Along a horizontal edge the step is up or down; along a vertical one, left or right.
    let horizontal=down>=across;
    var side=select(-px.x,-px.y,horizontal);
    if !horizontal { n=w; s=e; }
    var gradient=abs(n-m);
    var pair=(n+m)*0.5;
    if gradient<abs(s-m) { gradient=abs(s-m); pair=(s+m)*0.5; side=-side; }
    let threshold=gradient*0.25;
    let along=select(vec2(0.,px.y),vec2(px.x,0.),horizontal);
    let start=uv+select(vec2(side*0.5,0.),vec2(0.,side*0.5),horizontal);
    var to_n=start-along; var to_p=start+along;
    var end_n=0.; var end_p=0.;
    var done_n=false; var done_p=false;
    for (var i=0u; i<steps; i++) {
        if !done_n { end_n=fxaa_luma(surface_gl(to_n)); done_n=abs(end_n-pair)>=threshold; }
        if !done_n { to_n-=along; }
        if !done_p { end_p=fxaa_luma(surface_gl(to_p)); done_p=abs(end_p-pair)>=threshold; }
        if !done_p { to_p+=along; }
    }
    let far_n=select(uv.y-to_n.y,uv.x-to_n.x,horizontal);
    let far_p=select(to_p.y-uv.y,to_p.x-uv.x,horizontal);
    let nearer_n=far_n<far_p;
    let end=select(end_p,end_n,nearer_n);
    // Only the end whose luma lies across the pair from this pixel's moves it.
    if ((m-pair)<0.)==((end-pair)<0.) { side=0.; }
    let offset=(0.5-select(far_p,far_n,nearer_n)/(far_p+far_n)+subpixel*0.125)*side;
    let filtered=surface_gl(uv+select(vec2(offset,0.),vec2(0.,offset),horizontal));
    let filtered_luma=fxaa_luma(filtered)+1./(65536.*256.);
    return filtered*min(4.,mix(filtered_luma,average,subpixel)/filtered_luma);
}
@fragment fn present(q: Quad) -> @location(0) vec4<f32> {
    let uv=turned(q.uv);
    let level=u32(p.finish.y);
    var base=surface_at(uv);
    if level>0u { base=fxaa(uv,level); }
    let blurred=textureSampleLevel(previous,linear_clamp,uv,0.).rgb;
    let lum=display_luma(blurred);
    // Upstream left a TODO here: explicitly avoid division by zero on black.
    let colored=blurred/max(lum,0.000001)*pow(max(lum,0.),p.bloom.y);
    let result=base+colored*p.bloom.x;
    let shown=select(result,srgb_encode(result),p.processing.x>0.5);
    // Brightness at 0 leaves the picture exactly as it is; otherwise it works on the picture as
    // the game's final pass reads it, in 8-bit steps.
    let held=floor(clamp(shown,vec3(0.),vec3(1.))*255.+vec3(0.5))/255.;
    return vec4(select(color_pow(held,p.finish.x),shown,p.finish.x==1.),1.);
}
// The CRT off, as Super Win the Game draws without its CRT simulation: the signal pixel by pixel
// in its pixel aspect fitted inside the output, over black, whatever the overscan. Static, part
// of the game's picture, shows here too.
@fragment fn flat(q: Quad) -> @location(0) vec4<f32> {
    let picture=p.size.x/p.size.y*p.picture.x;
    let output=p.size.z/p.size.w;
    let fit=select(vec2(1.,picture/output),vec2(output/picture,1.),output>picture);
    let uv=(q.uv-vec2(0.5))*fit+vec2(0.5);
    if any(uv<vec2(0.)) || any(uv>=vec2(1.)) { return vec4(0.,0.,0.,1.); }
    let texel=(floor(uv*p.size.xy)+vec2(0.5))/p.size.xy;
    let c=picture_at(texel,static_strength());
    return vec4(select(c,srgb_decode(c),p.processing.x>0.5),1.);
}

// A frame handed to its host: the final picture, texel for texel, into the host's target. An
// sRGB target encodes what it is given, so it is given the colours decoded.
@fragment fn deliver(q: Quad) -> @location(0) vec4<f32> {
    return textureLoad(source,vec2<i32>(q.position.xy),0);
}
@fragment fn deliver_srgb(q: Quad) -> @location(0) vec4<f32> {
    let c=textureLoad(source,vec2<i32>(q.position.xy),0);
    return vec4(srgb_decode(c.rgb),1.);
}
