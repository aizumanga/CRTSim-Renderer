"""The CRT's glass and bezel per pixel, two ways, for measuring CRTSim-Renderer's approximations:

- `rasterize`: the meshes (`screen.m3d`, `frame.m3d`, byte-identical to the game's
  `Screen.m3d` and `Monitor2.m3d`) as the game draws them: each pixel's ray meets the nearest
  triangle, whose corners' attributes are blended at that point -- what GL's perspective-correct
  interpolation gives.
- `approximate`: `crtsim.wgsl`'s `surface()` ported line for line: the glass as a sphere, the
  bezel ray-marched across the images `bezel_maps` writes.

Both return a dict of HxW arrays: `kind` (0 nothing, 1 glass, 2 bezel), `uv` (HxWx2, the
mesh's convention: v down), `normal` (HxWx3), `shade` (the vertex colour's red, which both
meshes keep grey), `reflection`, and `point` (HxWx3, where the ray meets the surface).

The camera is the game's: at (-cot(fov/2), 0, 0) looking along +x, z up, -y to the right, with
the vertical field of view `fov`."""
import os, struct
import numpy as np
from PIL import Image

NOTHING = 1e9


def read_m3d(path):
    """`crtsim_core::mesh::Mesh::read`: positions, normals, colours (rgba 0-1), uvs, reflection."""
    d = open(path, 'rb').read()
    assert d[:4] == b'.m3d' and d[4:6] == bytes([2, 1])
    count, nv, ni = struct.unpack_from('<III', d, 8)
    at = 8 + 12 + 1
    assert struct.unpack_from('<I', d, at)[0] == 2
    at += 4
    indices = np.frombuffer(d, '<u2', ni, at).astype(int).reshape(-1, 3)
    at += 2 * ni
    mesh = {'reflection': np.zeros(nv)}
    for _ in range(count):
        usage, stride = struct.unpack_from('<II', d, at)
        at += 8
        if usage == 4:
            argb = np.frombuffer(d, '<u4', nv, at)
            mesh['color'] = np.stack([(argb >> 16) & 255, (argb >> 8) & 255, argb & 255,
                                      argb >> 24], 1) / 255.
        else:
            n = {0: 3, 1: 3, 5: 2, 6: 1}[usage]
            v = np.frombuffer(d, '<f4', nv * n, at).astype(float).reshape(nv, n)
            mesh[{0: 'position', 1: 'normal', 5: 'uv', 6: 'reflection'}[usage]] = v.reshape(nv, -1).squeeze()
        at += stride * nv
    assert at == len(d)
    mesh['indices'] = indices
    return mesh


def rays(width, height, fov):
    """Each pixel's camera and unit ray, through pixel centres, as `surface()` makes them."""
    cam = np.array([-1 / np.tan(np.radians(fov) / 2), 0., 0.])
    x = (np.arange(width) + 0.5) / width * 2 - 1
    y = 1 - (np.arange(height) + 0.5) / height * 2
    nx, ny = np.meshgrid(x, y)
    spread = -1 / cam[0]
    r = np.stack([np.ones_like(nx), -nx * spread * width / height, ny * spread], -1)
    return cam, r / np.linalg.norm(r, axis=-1, keepdims=True)


def empty(h, w):
    return {'kind': np.zeros((h, w), int), 'uv': np.zeros((h, w, 2)),
            'normal': np.zeros((h, w, 3)), 'shade': np.zeros((h, w)),
            'reflection': np.zeros((h, w)), 'point': np.zeros((h, w, 3)),
            'color': np.ones((h, w, 4)), 'depth': np.full((h, w), NOTHING)}


def rasterize(screen, frame, width, height, fov):
    """Nearest mesh surface per pixel, exactly: ray against triangle, attributes blended by the
    hit point's barycentric weights."""
    cam, ray = rays(width, height, fov)
    out = empty(height, width)
    f = 1 / np.tan(np.radians(fov) / 2)
    for kind, mesh in ((1, screen), (2, frame)):
        p = mesh['position']
        rel = p - cam
        # Where each vertex lands on screen, to bound the pixels a triangle can cover.
        sx = ((-rel[:, 1] / rel[:, 0]) * f * height / width + 1) / 2 * width
        sy = (1 - (rel[:, 2] / rel[:, 0]) * f) / 2 * height
        for tri in mesh['indices']:
            x0, x1 = int(np.floor(sx[tri].min())), int(np.ceil(sx[tri].max()))
            y0, y1 = int(np.floor(sy[tri].min())), int(np.ceil(sy[tri].max()))
            x0, y0, x1, y1 = max(x0, 0), max(y0, 0), min(x1, width - 1), min(y1, height - 1)
            if x0 > x1 or y0 > y1:
                continue
            a, b, c = p[tri]
            d = ray[y0:y1 + 1, x0:x1 + 1]
            # Moller-Trumbore
            e1, e2 = b - a, c - a
            pv = np.cross(d, e2)
            det = pv @ e1
            ok = np.abs(det) > 1e-12
            inv = np.where(ok, 1 / np.where(ok, det, 1), 0)
            tv = cam - a
            u = (pv @ tv) * inv
            qv = np.cross(tv, e1)
            v = (d @ qv) * inv
            t = (qv @ e2) * inv
            hit = ok & (u >= -1e-7) & (v >= -1e-7) & (u + v <= 1 + 1e-7) & (t > 0)
            point = cam + d * t[..., None]
            depth = point[..., 0]
            win = hit & (depth < out['depth'][y0:y1 + 1, x0:x1 + 1])
            if not win.any():
                continue
            w = np.stack([1 - u - v, u, v], -1)[win]
            sl = (slice(y0, y1 + 1), slice(x0, x1 + 1))
            out['depth'][sl][win] = depth[win]
            out['kind'][sl][win] = kind
            out['point'][sl][win] = point[win]
            out['uv'][sl][win] = w @ mesh['uv'][tri]
            n = w @ mesh['normal'][tri]
            out['normal'][sl][win] = n / np.linalg.norm(n, axis=-1, keepdims=True)
            col = w @ mesh['color'][tri]
            out['color'][sl][win] = col
            out['shade'][sl][win] = col[:, 0]
            out['reflection'][sl][win] = w @ mesh['reflection'][tri]
    return out


# --- crtsim.wgsl's surface() -------------------------------------------------------------

GLASS_CENTRE = np.array([4., 0., 0.])
GLASS_RADIUS = 4.
# Until session 7 the glass's outline and shade were a formula: a superellipse of this corner
# exponent, shaded 1 - 0.5 r^6. Since, they are baked from the mesh (crtsim-core's glass.rs).
GLASS_CORNER = 11.54
GLASS_EDGE = 1 / 64
BEZEL_HALF = np.array([1.6533334, 1.32])
BEZEL_NEAR, BEZEL_FAR = -0.1, 0.33
BEZEL_OPENING = np.array([1.24, 0.95])
BEZEL_STEPS, BEZEL_HALVINGS = 32, 7


class Bezel:
    def __init__(self, folder):
        load = lambda n: np.asarray(Image.open(f'{folder}/{n}.png').convert('RGBA'), float) / 255
        self.shape, self.uv, self.normal = load('shape'), load('uv'), load('normal')
        self.size = np.array([self.shape.shape[1], self.shape.shape[0]], float)
        self.glass = load('glass') if os.path.exists(f'{folder}/glass.png') else None

    def glass_baked(self, uv):
        """`glass_baked`: the mesh's shade and the distance to its outline at uv."""
        h, w = self.glass.shape[:2]
        g = np.clip(uv, 0, 1) * [w - 1, h - 1]
        xy = np.floor(g).astype(int)
        f = g - xy
        def dec(xy):
            v = self.load(self.glass, xy)
            return np.stack([self.pair(v[..., 0], v[..., 1]), self.pair(v[..., 2], v[..., 3])], -1)
        fx, fy = f[..., :1], f[..., 1:]
        v = (dec(xy) * (1 - fx) + dec(xy + [1, 0]) * fx) * (1 - fy) + \
            (dec(xy + [0, 1]) * (1 - fx) + dec(xy + [1, 1]) * fx) * fy
        return v[..., 0], -GLASS_EDGE + v[..., 1] * 2 * GLASS_EDGE

    def texel(self, yz):
        return (BEZEL_HALF - yz) / (2 * BEZEL_HALF) * self.size - 0.5

    def load(self, t, xy):
        h, w = t.shape[:2]
        return t[np.clip(xy[..., 1], 0, h - 1), np.clip(xy[..., 0], 0, w - 1)]

    @staticmethod
    def pair(hi, lo):
        return (np.round(hi * 255) * 256 + np.round(lo * 255)) / 65534

    def depth_at(self, xy):
        h, w = self.shape.shape[:2]
        tx = self.load(self.shape, xy)
        empty = ((xy < 0).any(-1) | (xy[..., 0] >= w) | (xy[..., 1] >= h)
                 | ((tx[..., 0] > 0.999) & (tx[..., 1] > 0.999)))
        d = BEZEL_NEAR + (BEZEL_FAR - BEZEL_NEAR) * self.pair(tx[..., 0], tx[..., 1])
        return np.where(empty, NOTHING, d)

    def corners(self, yz):
        t = self.texel(yz)
        xy = np.floor(t).astype(int)
        return xy, t - xy

    def depth(self, yz):
        xy, f = self.corners(yz)
        a, b = self.depth_at(xy), self.depth_at(xy + [1, 0])
        c, d = self.depth_at(xy + [0, 1]), self.depth_at(xy + [1, 1])
        fx, fy = f[..., 0], f[..., 1]
        blended = (a * (1 - fx) + b * fx) * (1 - fy) + (c * (1 - fx) + d * fx) * fy
        return np.where(np.maximum(np.maximum(a, b), np.maximum(c, d)) >= NOTHING, NOTHING, blended)

    def pairs(self, t, yz):
        xy, f = self.corners(yz)
        def dec(xy):
            v = self.load(t, xy)
            return np.stack([self.pair(v[..., 0], v[..., 1]), self.pair(v[..., 2], v[..., 3])], -1)
        fx, fy = f[..., :1], f[..., 1:]
        return (dec(xy) * (1 - fx) + dec(xy + [1, 0]) * fx) * (1 - fy) + \
               (dec(xy + [0, 1]) * (1 - fx) + dec(xy + [1, 1]) * fx) * fy

    def bytes(self, yz):
        xy, f = self.corners(yz)
        g = lambda xy: self.load(self.shape, xy)[..., 2:]
        fx, fy = f[..., :1], f[..., 1:]
        return (g(xy) * (1 - fx) + g(xy + [1, 0]) * fx) * (1 - fy) + \
               (g(xy + [0, 1]) * (1 - fx) + g(xy + [1, 1]) * fx) * fy

    def coarse(self, at):
        """textureSampleLevel(shape, linear_clamp, at).r: bilinear, clamped to the edge."""
        h, w = self.shape.shape[:2]
        p = at * [w, h] - 0.5
        xy = np.floor(p).astype(int)
        f = p - xy
        r = lambda xy: self.load(self.shape, xy)[..., 0]
        fx, fy = f[..., 0], f[..., 1]
        return (r(xy) * (1 - fx) + r(xy + [1, 0]) * fx) * (1 - fy) + \
               (r(xy + [0, 1]) * (1 - fx) + r(xy + [1, 1]) * fx) * fy


def approximate(bezel, width, height, fov, screen_only=False, formula=False):
    """`surface()`: the sphere's glass and the ray-marched bezel, nearest wins. `formula` draws
    the glass's outline and shade as the renderer did until session 7."""
    cam, ray = rays(width, height, fov)
    out = empty(height, width)
    along = lambda x: cam + ray * ((x - cam[0]) / ray[..., 0])[..., None]
    to = cam - GLASS_CENTRE
    b = ray @ to
    reach = b * b - to @ to + GLASS_RADIUS ** 2
    glass_at = cam + ray * (-b - np.sqrt(np.maximum(reach, 0)))[..., None]
    guv = np.stack([(4 / 3 - glass_at[..., 1]) * 0.375, (1 - glass_at[..., 2]) * 0.5], -1)
    if formula:
        edge = np.abs(guv * 2 - 1)
        radius = (edge[..., 0] ** GLASS_CORNER + edge[..., 1] ** GLASS_CORNER) ** (1 / GLASS_CORNER)
        shade, outside = 1 - 0.5 * radius ** 6, radius > 1
    else:
        shade, distance = bezel.glass_baked(guv)
        outside = (guv < 0).any(-1) | (guv > 1).any(-1) | (distance < 0)
    glass_depth = np.where((reach < 0) | outside, NOTHING, glass_at[..., 0])

    bezel_depth = np.full((height, width), NOTHING)
    if not screen_only:
        near, far = np.abs(along(BEZEL_NEAR)[..., 1:]), np.abs(along(BEZEL_FAR)[..., 1:])
        may = (np.maximum(near, far) >= BEZEL_OPENING).any(-1)
        step = (BEZEL_FAR - BEZEL_NEAR) / BEZEL_STEPS
        before = np.full((height, width), BEZEL_NEAR - step)
        after = np.full((height, width), NOTHING)
        searching = may.copy()
        for i in range(BEZEL_STEPS + 1):
            x = BEZEL_NEAR + step * i
            at = (bezel.texel(along(x)[..., 1:]) + 0.5) / bezel.size
            coarse = BEZEL_NEAR + (BEZEL_FAR - BEZEL_NEAR) * bezel.coarse(at) * 255 * 256 / 65534
            hit = searching & (x >= coarse) & (at >= 0).all(-1) & (at <= 1).all(-1)
            after = np.where(hit, x, after)
            before = np.where(searching & ~hit, x, before)
            searching &= ~hit
        low, high = before.copy(), after.copy()
        for _ in range(BEZEL_HALVINGS):
            middle = 0.5 * (low + high)
            inside = middle >= bezel.depth(along(middle)[..., 1:])
            high = np.where(inside, middle, high)
            low = np.where(inside, low, middle)
        found = may & (after < NOTHING) & (bezel.depth(along(high)[..., 1:]) < NOTHING)
        bezel_depth = np.where(found, high, NOTHING)
    bezel_at = along(np.minimum(bezel_depth, BEZEL_FAR))

    is_bezel = bezel_depth < glass_depth
    is_glass = ~is_bezel & (glass_depth < NOTHING)
    out['kind'][is_glass], out['kind'][is_bezel] = 1, 2
    out['uv'][is_glass] = guv[is_glass]
    out['normal'][is_glass] = ((glass_at - GLASS_CENTRE) / GLASS_RADIUS)[is_glass]
    out['shade'][is_glass] = shade[is_glass]
    out['color'][is_glass] = np.stack([shade] * 3 + [np.ones_like(shade)], -1)[is_glass]
    out['point'][is_glass] = glass_at[is_glass]
    out['depth'][is_glass] = glass_depth[is_glass]
    if is_bezel.any():
        yz = bezel_at[is_bezel][:, 1:]
        out['uv'][is_bezel] = bezel.pairs(bezel.uv, yz)
        facing = bezel.pairs(bezel.normal, yz) * 2 - 1
        nx = -np.sqrt(np.maximum(1 - (facing ** 2).sum(-1), 0))
        out['normal'][is_bezel] = np.concatenate([nx[:, None], facing], -1)
        by = bezel.bytes(yz)
        out['shade'][is_bezel] = by[:, 0]
        out['color'][is_bezel] = np.stack([by[:, 0]] * 3 + [np.ones(len(by))], -1)
        out['reflection'][is_bezel] = by[:, 1]
        out['point'][is_bezel] = bezel_at[is_bezel]
        out['depth'][is_bezel] = bezel_depth[is_bezel]
    return out


# --- shading: screen.fx and monitor.fx -----------------------------------------------------

GAME = dict(uv_scale=0.9795918, overscan=1., barrel=-0.115, satur=1.35, dimming=0.5,
            refl=0.3, diffuse=0.5, spec=0.35, spec_power=50., fresnel=1.,
            light=np.array([-10., -5., 10.]), monitor=np.array([0.06, 0.06, 0.06, 0.]),
            mask_scale=np.array([128., 224.]), mask_bright=0.45, mask_opacity=1.)


def bilinear_border(image, uv):
    """compFrameMap: bilinear, black outside. `image` HxWx3 rows top first; uv v down."""
    h, w = image.shape[:2]
    p = uv * [w, h] - 0.5
    xy = np.floor(p).astype(int)
    f = p - xy
    def at(dx, dy):
        x, y = xy[..., 0] + dx, xy[..., 1] + dy
        inside = (x >= 0) & (x < w) & (y >= 0) & (y < h)
        return np.where(inside[..., None], image[np.clip(y, 0, h - 1), np.clip(x, 0, w - 1)], 0.)
    fx, fy = f[..., :1], f[..., 1:]
    return (at(0, 0) * (1 - fx) + at(1, 0) * fx) * (1 - fy) + (at(0, 1) * (1 - fx) + at(1, 1) * fx) * fy


def mask_sample(levels, uv):
    """scanlinesMap: wrapped, trilinear across `levels` (each the box average of the one
    above), the level of detail from how fast `uv` changes from pixel to pixel."""
    h0, w0 = levels[0].shape[:2]
    du = np.gradient(uv[..., 0] * w0, axis=(0, 1))
    dv = np.gradient(uv[..., 1] * h0, axis=(0, 1))
    rho = np.maximum(np.hypot(du[1], dv[1]), np.hypot(du[0], dv[0]))
    lod = np.clip(np.log2(np.maximum(rho, 1e-6)), 0, len(levels) - 1)
    lo = np.floor(lod).astype(int)
    out = np.zeros(uv.shape[:2] + (3,))
    for level in range(len(levels)):
        img = levels[level]
        h, w = img.shape[:2]
        p = uv * [w, h] - 0.5
        xy = np.floor(p).astype(int)
        f = p - xy
        at = lambda dx, dy: img[(xy[..., 1] + dy) % h, (xy[..., 0] + dx) % w]
        fx, fy = f[..., :1], f[..., 1:]
        s = (at(0, 0) * (1 - fx) + at(1, 0) * fx) * (1 - fy) + (at(0, 1) * (1 - fx) + at(1, 1) * fx) * fy
        weight = np.where(lo == level, 1 - (lod - lo), 0) + np.where(lo + 1 == level, lod - lo, 0)
        out += s * weight[..., None]
    return out


def mask_levels(mask):
    levels = [mask]
    while levels[-1].shape[0] > 1 or levels[-1].shape[1] > 1:
        m = levels[-1]
        h, w = max(m.shape[0] // 2, 1), max(m.shape[1] // 2, 1)
        m = m[:h * 2 if m.shape[0] > 1 else 1, :w * 2 if m.shape[1] > 1 else 1]
        levels.append(m.reshape(h, m.shape[0] // h, w, m.shape[1] // w, 3).mean((1, 3)))
    return levels


def sample_crt(uv, comp, levels, p):
    scaled = uv * [1, p['uv_scale']] + [0, (1 - p['uv_scale']) / 2]
    if p['mask_opacity'] > 0:
        scan = mask_sample(levels, scaled * p['mask_scale']) + p['mask_bright']
        scan = 1 + (scan - 1) * p['mask_opacity']
    else:
        scan = 1.
    o = p['overscan']
    q = scaled * o - (o - 1) * 0.5 - 0.5
    q = q + q * (p['barrel'] * (q ** 2).sum(-1))[..., None] + 0.5
    e = bilinear_border(comp, q) * scan
    desat = e @ np.array([0.299, 0.587, 0.114])
    return desat[..., None] + (e - desat[..., None]) * p['satur']


def shade(s, comp, levels, fov, p=GAME):
    """The game's screen.fx on the glass and monitor.fx on the bezel, per pixel of `s`."""
    cam = np.array([-1 / np.tan(np.radians(fov) / 2), 0., 0.])
    unit = lambda v: v / np.maximum(np.linalg.norm(v, axis=-1, keepdims=True), 1e-12)
    n = unit(s['normal'])
    c = unit(cam - s['point'])
    l = unit(p['light'] - s['point'])
    diffuse = np.clip((n * l).sum(-1), 0, 1)[..., None]
    spec = (np.clip((n * unit(l + c)).sum(-1), 0, 1) ** p['spec_power'])[..., None] * 0.25 * p['spec']
    fres = ((1 - (c * n).sum(-1)) ** 2)[..., None] * p['fresnel']
    emissive = sample_crt(s['uv'], comp, levels, p)
    glass = fres * [0.45, 0.4, 0.5] + diffuse * [0.175, 0.15, 0.2] * p['diffuse'] + spec + emissive
    hemi = ((n[..., 2] * 0.5 + 0.5) * 0.4 + 0.3)[..., None]
    bezel = (fres * 0.15 + p['monitor'][:3] * (diffuse + hemi) * p['diffuse'] + spec
             + emissive * s['reflection'][..., None] * p['refl'])
    rgb = np.where((s['kind'] == 1)[..., None], glass, bezel)
    rgb = rgb * (1 + (s['color'][..., :3] - 1) * p['dimming'])
    rgb = np.where((s['kind'] == 0)[..., None], 0., rgb)
    return np.clip(np.floor(np.clip(rgb, 0, 1) * 255 + 0.5), 0, 255).astype(np.uint8)
