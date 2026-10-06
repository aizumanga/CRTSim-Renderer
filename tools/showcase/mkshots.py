import json, sys
S = sys.argv[1]
O = "1080x810"
shots = {}
def shot(name, base, config, source, frames, **kw):
    d = dict(base=base, config=dict(dict(output=O), **config), source=source, frames=frames, fps=30, out=f"{S}/renders/{name}")
    d.update(kw); shots[name] = d
clip = lambda start: {"test_clip": start, "step": 2}
synth = lambda start: {"dir": f"{S}/src/synth", "dir_start": start}
shot("title", "default", {"fov": 38}, {"dir": f"{S}/src/title"}, 120)
shot("compare", "default", {}, clip(0), 180)
shot("compare_raw", "default", {}, clip(0), 180, dump_source=True)
shot("macro", "default", {"output": "2880x2160"}, clip(360), 120)
shot("m1_original", "default", {}, clip(120), 30)
shot("m2_superwin", "default", {"fov": 30, "ntsc_blending": 0.35, "reflection_as_screen": True, "backdrop_color": [0.0625]*3, "palette": {}}, clip(180), 30)
shot("m3_soft", "general", {"mask_opacity": 0.45, "artifacts": 0.2, "sharpness": 0.25, "bloom": 0.16}, synth(0), 30)
shot("m4_ntsc240", "general", {"signal": "240p", "filter": "nearest", "mask_opacity": 0.7}, clip(240), 30, mask_signal=True)
shot("m5_pal576i", "general", {"signal": "576p", "interlace": True, "artifacts": 0.25}, synth(30), 30, mask_signal=True)
shot("m6_warm", "general", {"hue": -8, "chroma": 0.85, "artifacts": 0.65, "mask_opacity": 0.65}, clip(300), 30)
shot("m7_clean", "general", {"artifacts": 0, "sharpness": 0, "bleed": 0, "persistence": [0,0,0], "mask_opacity": 0.55, "bloom": 0.08}, synth(60), 30)
shot("m8_linear", "general", {"color_mode": "linear-light", "bloom": 0.08, "diffuse": 0.12, "specular": 0.08, "rim": 0.25, "mask_opacity": 0.65}, clip(420), 30)
shot("pullback", "default", {"specular": 0.6}, clip(480), 120, animate=[
    {"path": "fov", "from": 12, "to": 44, "ease": "inout", "start": 0.05, "end": 0.85},
    {"path": "barrel", "from": -0.38, "to": -0.115, "ease": "inout", "start": 0.0, "end": 0.8},
    {"path": "light_position.0", "from": -14, "to": 14, "ease": "inout"},
    {"path": "bloom", "from": 0.6, "to": 0.25, "ease": "out", "start": 0, "end": 0.5},
])
shot("outro", "default", {"fov": 38}, {"dir": f"{S}/src/outro"}, 120)
order = ["title","compare_raw","compare","m1_original","m2_superwin","m3_soft","m4_ntsc240","m5_pal576i","m6_warm","m7_clean","m8_linear","pullback","outro","macro"]
for n in order:
    json.dump(shots[n], open(f"{S}/shots/{n}.json","w"), indent=1)
open(f"{S}/shots/order.txt","w").write("\n".join(order))
