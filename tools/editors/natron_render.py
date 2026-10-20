# Renders frames through the OpenFX effect in NatronRenderer, for tools/check_editors.py:
# NatronRenderer natron_render.py, with the frames, output and settings in the environment.
import os

app = app1  # noqa: F821, NatronRenderer's project
read = app.createNode("fr.inria.openfx.ReadOIIO")
read.getParam("filename").setValue(os.environ["CRTSIM_INPUT"])
read.getParam("firstFrame").setValue(int(os.environ["CRTSIM_INPUT_FIRST"]))
read.getParam("lastFrame").setValue(int(os.environ["CRTSIM_INPUT_LAST"]))
crt = app.createNode("io.github.aizumanga.CRTSimRenderer")
crt.getParam("timing").setValue(int(os.environ.get("CRTSIM_TIMING", "0")))
crt.connectInput(0, read)
write = app.createNode("fr.inria.openfx.WriteOIIO")
write.getParam("filename").setValue(os.environ["CRTSIM_OUTPUT"])
first, last = int(os.environ["CRTSIM_FIRST"]), int(os.environ["CRTSIM_LAST"])
write.getParam("frameRange").setValue(2)
write.getParam("firstFrame").setValue(first)
write.getParam("lastFrame").setValue(last)
write.connectInput(0, crt)
app.render(write, first, last)
