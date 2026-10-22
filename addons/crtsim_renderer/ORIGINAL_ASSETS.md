# Upstream provenance

- Repository: https://github.com/MinorKeyGames/CRTSim
- Pinned commit: `dbbe9d1bc2512288f5b1747e6be35ff44a7baae1`
- Author: J. Kyle Pittman
- License: CC0 1.0; original dedication in COPYING.txt
- The .bmp, .m3d and .fx files are byte-for-byte copies from `CRTSim/bin/`.
- The reader follows the published `CRTSim/src/M3D.cpp` and `M3D.h` format, with added bounds/version validation.
- No files were extracted from either commercial game, and no unpublished shader is assumed.

SHA-256 values (verify with `python3 tools/verify_assets.py`):

```
b18ac6e52bbaf597cfcd20831a2354156b6b1ffa1675ecda0a425bdf78cb2d8f COPYING.txt
d9d52bf6f90f0daf9d56f4da396a84a234113e90eadea385acc75c1c52fbfaa4 artifacts.bmp
9854a591fd0044fff54d95ea25ea5acc5743d720d294a115475441e460d9cdd1 composite.fx
b0b2c29f7604099eba5594b31ad61b1d3d352b8bafdf85f9fc0c4b8b2ee340b3 crtbase.fx
97e8592006ca624f6f1302b4b0d14a487982e79ad2d6f56927f388c5147538f1 frame.fx
f6e07a7a8cc39e3eabf2259e87415e2bf0cec4ad540a5522ea781c953fd05e07 frame.m3d
2e675bf04ac5c306d75cfbb5b3df3b6bd832ce682f82b3c3bcca3ceec7cd0116 mask.bmp
48010cd935e5f92a620c86f319364ccb22283e891b2b9888a71a76cf8df2a024 post.fx
eac4db1317751688eb66d48b4f5a9ed5c6402f8a1b6f7fe44cdd4aa5ceb8855e present.fx
d578d7f7fc463fc77d34d031c8f9a66eff89f1c156ee127e021634839f0437b6 screen.fx
d279b240c908d040183676fdd4ad4bbccdf33325ba7b9572229e8f752f0bb43b screen.m3d
```
