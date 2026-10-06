"""usage: compare_palette.py PALETTE_BIN  (palette.bin from capture_ntsc_lut.sh)
Compare CRTSim-Renderer's NesPalette::default().colors() (ported verbatim from palette.rs)
with the palette SWTG's PaletteGen::MakePalette produced at its defaults."""
import math, sys
def decode(index):
    LOW=[0.350,0.518,0.962,1.550]; HIGH=[1.094,1.506,1.962,1.962]; BLACK=0.518; WHITE=1.962
    hue=index%16; level=1 if hue>13 else index//16
    low,high=LOW[level],HIGH[level]
    if hue==0: low=high
    if hue>12: high=low
    y=re=im=0.
    for phase in range(12):
        volts=high if (hue+phase)%12<6 else low
        s=(volts-BLACK)/(WHITE-BLACK); a=math.pi*phase/6
        y+=s/12; re+=s*math.cos(a)*2/12; im-=s*math.sin(a)*2/12
    turn=math.pi-math.pi/6*5.5+math.radians(-8.); sn,cs=math.sin(turn),math.cos(turn)
    return y,(re*cs-im*sn)*0.6,(re*sn+im*cs)*0.6
def renderer_colors():
    out=[]
    for i in range(64):
        y,u,v=decode(i)
        c33,s33=math.cos(math.radians(33)),math.sin(math.radians(33))
        I=(v*c33-u*s33); Q=(v*s33+u*c33)
        u,v=(Q*c33-I*s33, Q*s33+I*c33)
        b,r=y+u/0.492,y+v/0.877; g=(y-0.299*r-0.114*b)/0.587
        out.append([round(max(0,min(1,c))*255) for c in (r,g,b)])
    return out
pal=open(sys.argv[1],'rb').read()
game=[list(pal[i*4:i*4+3]) for i in range(64)]
ren=renderer_colors()
diffs=[]
for i in range(64):
    if (i%16) in (14,15) or i==0x0D: continue  # blacks
    d=sum(abs(a-b) for a,b in zip(game[i],ren[i]))/3; diffs.append((d,i))
print('mean abs diff over %d non-black entries: %.1f steps' % (len(diffs), sum(d for d,_ in diffs)/len(diffs)))
print('worst:', ', '.join('%02X game=%s renderer=%s' % (i, game[i], ren[i]) for d,i in sorted(diffs)[-5:]))
print('sample 0x00/0x16/0x2A/0x12:', [(hex(i), game[i], ren[i]) for i in (0x00,0x16,0x2A,0x12)])
