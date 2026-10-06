# Runs the game's PaletteGen::MakePalette on Tint/I/Q settings, stopped at main, and saves each
# 256-entry palette as JSON. From the game's directory (a copy):
#   ORACLE_OUT=out.json N_PAL=53 LD_LIBRARY_PATH=. gdb -q -batch -x palette_oracle.py ./SuperGame_NFML
# FromYIQ is not called directly: it returns through a hidden pointer it pops itself (ret 4),
# and gdb then misses the call's end and lets the game run on.
import gdb, json, random, struct, os
OUT = os.environ['ORACLE_OUT']
gdb.execute('set pagination off'); gdb.execute('set confirm off')
gdb.execute('break main'); gdb.execute('run')
def ev(e): return gdb.parse_and_eval(e)
inf = gdb.selected_inferior()
buf = int(ev('(unsigned int)malloc(1024)'))
def f32(x): return struct.unpack('<f', struct.pack('<f', x))[0]
rng = random.Random(1234)
settings = [(5.183186, 1.75, 1.0), (0.0, 1.0, 1.0), (3.0, 0.25, 4.0), (6.283185, 4.0, 0.25), (1.234, 2.5, 1.5), (5.5, 1.75, 1.0), (4.9, 1.0, 2.0)]
settings += [(f32(rng.uniform(0, 6.283185)), f32(rng.uniform(0.25, 4)), f32(rng.uniform(0.25, 4))) for _ in range(int(os.environ.get("N_PAL", "20")))]
pals = []
for (t, i, q) in settings:
    for k in range(256): inf.write_memory(buf + 4*k, struct.pack('<I', 0xff000000))
    gdb.execute('call ((void(*)(void*,float,float,float))0x080c8a10)((void*)%d, %rf, %rf, %rf)' % (buf, t, i, q), to_string=True)
    pals.append(list(bytes(inf.read_memory(buf, 1024))))
json.dump({'settings': settings, 'palettes': pals}, open(OUT, 'w'))
gdb.execute('kill'); gdb.execute('quit')
