// CC0. An LD_PRELOAD library for Super Win the Game's Linux build that feeds a test image into
// its CRT pipeline and reads back what each CRT pass draws, for comparing pass by pass.
//
// Build: gcc -m32 -shared -fPIC -O2 -o capture_shim.so capture_shim.c -ldl
// Run:   SHIM_DIR=out LD_PRELOAD=$PWD/capture_shim.so ./SuperGame_NFML
//
// The game draws with glDrawElements, which it imports, and loads GL 2.0 through
// glXGetProcAddressARB, which this wraps. Shader programs are told apart by uniforms only one
// CRT pass declares. Until SHIM_DIR/go exists the game runs untouched. The file holds lines:
//   input <path>   a 256x224 RGBA image, rows top first, raw bytes; without it the scene's
//                  own frames are recorded as the game draws them
//   frames <n>     frames to capture, counted in NTSC passes
//   skip <n>       frames to let pass first, so the glow of the scene before has decayed
//   tag <name>     prefix for the files written
// and, since session 11:
//   only <a,b,..>  save pixels only for these passes (ntsc,composite,screen,monitor,bloomdown,
//                  bloomup,compose); uniforms (.txt) are still written for every pass
//   clearscreen 1  clear the target to magenta before the screen pass, so the screen pass's
//                  readback holds the glass alone and the monitor's glass + bezel (masks)
//   logall 1       log every draw of the captured frames, any program, with its effect name
//                  (from the GLSL's "Input source:" comment), target and uniforms, and every
//                  glGenerateMipmap and glTexParameteri, to <tag>_<frame>_draws.txt
//   readfx <a,b>   also read back what draws of these effects (e.g. static,tuner,wipe) drew,
//                  to <tag>_<frame>_d<draw>_<fx>.rgba with a .txt
//   mipdump 1      at the bloom downsample, write the texture it samples as PreHUDBuffer: its
//                  filters, levels' sizes and level 0-6 pixels (glGetTexImage) to
//                  <tag>_<frame>_prehud.txt and <tag>_<frame>_prehud_L<n>.rgba
// From then on, before every NTSC pass the Clean Frame texture is replaced with the input, and
// for the first n frames each CRT pass's target is read back to SHIM_DIR/<tag>_<frame>_<pass>
// .rgba (rows bottom first, as GL reads them) with its size and uniforms in a .txt beside it.
// Changing go's contents starts again, with the options it does not give off.
#define _GNU_SOURCE
#include <GL/gl.h>
#include <dlfcn.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <sys/stat.h>

typedef char GLchar_;
enum { NONE, NTSC, COMPOSITE, SCREEN, MONITOR, BLOOM_DOWN, BLOOM_UP, COMPOSE, PASSES };
static const char *pass_name[PASSES] = {"none", "ntsc", "composite", "screen", "monitor",
                                        "bloomdown", "bloomup", "compose"};

#define MAX_IDS 4096
static int shader_kind_hint[MAX_IDS];   // per shader: the pass its source marks, if any
static int program_kind[MAX_IDS];       // per program
static int current_program;
static char *uniform_name[MAX_IDS][64]; // per program, by location
static char uniform_value[MAX_IDS][64][160];

static void *(*real_getproc)(const GLubyte *);
static void (*real_draw_elements)(GLenum, GLsizei, GLenum, const void *);
static void (*gl_shader_source)(GLuint, GLsizei, const GLchar_ *const *, const GLint *);
static void (*gl_attach_shader)(GLuint, GLuint);
static void (*gl_link_program)(GLuint);
static void (*gl_use_program)(GLuint);
static GLint (*gl_get_uniform_location)(GLuint, const GLchar_ *);
static void (*gl_get_uniformiv)(GLuint, GLint, GLint *);
static void (*gl_active_texture)(GLenum);
static void (*gl_tex_sub_image)(GLenum, GLint, GLint, GLint, GLsizei, GLsizei, GLenum, GLenum,
                                const void *);
static void (*gl_read_pixels)(GLint, GLint, GLsizei, GLsizei, GLenum, GLenum, void *);
static void (*gl_get_integerv)(GLenum, GLint *);
static void (*gl_bind_texture)(GLenum, GLuint);
static void (*gl_bind_framebuffer)(GLenum, GLuint);
static void (*gl_uniform1f)(GLint, GLfloat);
static void (*gl_uniform1i)(GLint, GLint);
static void (*gl_uniform1fv)(GLint, GLsizei, const GLfloat *);
static void (*gl_uniform2fv)(GLint, GLsizei, const GLfloat *);
static void (*gl_uniform3fv)(GLint, GLsizei, const GLfloat *);
static void (*gl_uniform4fv)(GLint, GLsizei, const GLfloat *);
static void (*gl_uniform_matrix4fv)(GLint, GLsizei, GLboolean, const GLfloat *);
static void (*gl_uniform2f)(GLint, GLfloat, GLfloat);
static void (*gl_uniform3f)(GLint, GLfloat, GLfloat, GLfloat);
static void (*gl_uniform4f)(GLint, GLfloat, GLfloat, GLfloat, GLfloat);

static void *libgl(const char *name) {
    static void *handle;
    if (!handle) handle = dlopen("libGL.so.1", RTLD_LAZY | RTLD_GLOBAL);
    void *f = real_getproc ? real_getproc((const GLubyte *)name) : NULL;
    return f ? f : dlsym(handle, name);
}

// --- shaders and programs ---------------------------------------------------------------

static char *shader_fx[MAX_IDS], *program_fx[MAX_IDS];
static void my_shader_source(GLuint shader, GLsizei count, const GLchar_ *const *text,
                             const GLint *length) {
    int kind = NONE;
    for (int i = 0; i < count && shader < MAX_IDS; i++) {
        const char *t = text[i];
        const char *src = strstr(t, "Input source: ");
        if (src && !shader_fx[shader]) {
            src += 14;
            size_t n = strcspn(src, ". \r\n");
            shader_fx[shader] = strndup(src, n);
        }
        if (strstr(t, "GradingRes")) kind = NTSC;
        else if (strstr(t, "NTSCHackTex")) kind = COMPOSITE;
        else if (strstr(t, "MonitorColor")) kind = MONITOR;
        else if (strstr(t, "scanlinesMap")) kind = SCREEN;
        else if (strstr(t, "PauseOpacity")) kind = COMPOSE;
        else if (strstr(t, "Poisson") && strstr(t, "tex2D (PreHUDBuffer")) kind = BLOOM_DOWN;
        else if (strstr(t, "Poisson") && strstr(t, "tex2D (DownsampleBuffer")) kind = BLOOM_UP;
    }
    if (shader < MAX_IDS && kind != NONE) shader_kind_hint[shader] = kind;
    gl_shader_source(shader, count, text, length);
}
static int attached_kind[MAX_IDS];
static void my_attach_shader(GLuint program, GLuint shader) {
    if (program < MAX_IDS && shader < MAX_IDS && shader_kind_hint[shader])
        attached_kind[program] = shader_kind_hint[shader];
    if (program < MAX_IDS && shader < MAX_IDS && shader_fx[shader]) program_fx[program] = shader_fx[shader];
    gl_attach_shader(program, shader);
}
static void my_link_program(GLuint program) {
    gl_link_program(program);
    if (program < MAX_IDS) program_kind[program] = attached_kind[program];
}
static void my_use_program(GLuint program) {
    current_program = program;
    gl_use_program(program);
}
static GLint my_get_uniform_location(GLuint program, const GLchar_ *name) {
    GLint loc = gl_get_uniform_location(program, name);
    if (program < MAX_IDS && loc >= 0 && loc < 64 && !uniform_name[program][loc])
        uniform_name[program][loc] = strdup(name);
    return loc;
}

// --- uniforms, remembered per program as text -------------------------------------------

static char *slot(GLint loc) {
    if (current_program <= 0 || current_program >= MAX_IDS || loc < 0 || loc >= 64) return NULL;
    return uniform_value[current_program][loc];
}
static void floats(GLint loc, int n, const GLfloat *v) {
    char *s = slot(loc);
    if (!s) return;
    int at = 0;
    for (int i = 0; i < n && at < 150; i++) at += snprintf(s + at, 160 - at, "%s%.9g", i ? " " : "", v[i]);
}
static void my_uniform1f(GLint l, GLfloat v) { floats(l, 1, &v); gl_uniform1f(l, v); }
static void my_uniform1i(GLint l, GLint v) {
    char *s = slot(l);
    if (s) snprintf(s, 160, "%d", v);
    gl_uniform1i(l, v);
}
static void my_uniform1fv(GLint l, GLsizei c, const GLfloat *v) { floats(l, c, v); gl_uniform1fv(l, c, v); }
static void my_uniform2fv(GLint l, GLsizei c, const GLfloat *v) { floats(l, 2 * c, v); gl_uniform2fv(l, c, v); }
static void my_uniform3fv(GLint l, GLsizei c, const GLfloat *v) { floats(l, 3 * c, v); gl_uniform3fv(l, c, v); }
static void my_uniform4fv(GLint l, GLsizei c, const GLfloat *v) { floats(l, 4 * c, v); gl_uniform4fv(l, c, v); }
static void my_uniform2f(GLint l, GLfloat x, GLfloat y) {
    GLfloat v[] = {x, y};
    floats(l, 2, v);
    gl_uniform2f(l, x, y);
}
static void my_uniform3f(GLint l, GLfloat x, GLfloat y, GLfloat z) {
    GLfloat v[] = {x, y, z};
    floats(l, 3, v);
    gl_uniform3f(l, x, y, z);
}
static void my_uniform4f(GLint l, GLfloat x, GLfloat y, GLfloat z, GLfloat w) {
    GLfloat v[] = {x, y, z, w};
    floats(l, 4, v);
    gl_uniform4f(l, x, y, z, w);
}
static void my_uniform_matrix4fv(GLint l, GLsizei c, GLboolean t, const GLfloat *v) {
    floats(l, 16 * c, v);
    gl_uniform_matrix4fv(l, c, t, v);
}

// --- the test --------------------------------------------------------------------------

static char go_text[4096];
static unsigned char *input;   // 256x224 RGBA, rows bottom first for GL
static int injecting, capturing, frames_wanted, frames_skipped, frame;
static int clear_screen, log_all, draw_no, mip_dump;
static char only[512], readfx[512];
static char tag[256] = "capture";

static void check_go(void) {
    const char *dir = getenv("SHIM_DIR");
    if (!dir) return;
    char path[4096], text[4096] = {0};
    snprintf(path, sizeof path, "%s/go", dir);
    FILE *f = fopen(path, "r");
    if (!f) return;
    size_t n = fread(text, 1, sizeof text - 1, f);
    fclose(f);
    text[n] = 0;
    if (!strcmp(text, go_text)) return;
    strcpy(go_text, text);
    char *line = strtok(text, "\n");
    char image[4096] = {0};
    only[0] = readfx[0] = 0;
    clear_screen = log_all = mip_dump = 0;
    for (; line; line = strtok(NULL, "\n")) {
        if (!strncmp(line, "input ", 6)) snprintf(image, sizeof image, "%s", line + 6);
        else if (!strncmp(line, "frames ", 7)) frames_wanted = atoi(line + 7);
        else if (!strncmp(line, "skip ", 5)) frames_skipped = atoi(line + 5);
        else if (!strncmp(line, "tag ", 4)) snprintf(tag, sizeof tag, "%s", line + 4);
        else if (!strncmp(line, "only ", 5)) snprintf(only, sizeof only, ",%s,", line + 5);
        else if (!strncmp(line, "readfx ", 7)) snprintf(readfx, sizeof readfx, ",%s,", line + 7);
        else if (!strncmp(line, "clearscreen ", 12)) clear_screen = atoi(line + 12);
        else if (!strncmp(line, "logall ", 7)) log_all = atoi(line + 7);
        else if (!strncmp(line, "mipdump ", 8)) mip_dump = atoi(line + 8);
    }
    frame = -frames_skipped;
    capturing = 1;
    injecting = 0;
    if (!image[0]) { fprintf(stderr, "shim: %s, recording %d frames\n", tag, frames_wanted); return; }
    FILE *img = fopen(image, "rb");
    if (!img) { fprintf(stderr, "shim: cannot open %s\n", image); return; }
    unsigned char *top_first = malloc(256 * 224 * 4);
    if (fread(top_first, 1, 256 * 224 * 4, img) != 256 * 224 * 4) fprintf(stderr, "shim: short image\n");
    fclose(img);
    free(input);
    input = malloc(256 * 224 * 4);
    for (int y = 0; y < 224; y++) memcpy(input + y * 1024, top_first + (223 - y) * 1024, 1024);
    free(top_first);
    injecting = 1;
    fprintf(stderr, "shim: %s, %d frames from %s\n", tag, frames_wanted, image);
}

static int listed(const char *list, const char *name) {
    char key[128];
    snprintf(key, sizeof key, ",%s,", name);
    return list[0] && strstr(list, key);
}
static void save_named(const char *name, int pixels_too);
static void save(int kind) { save_named(pass_name[kind], !only[0] || listed(only, pass_name[kind])); }
static void save_named(const char *name, int pixels_too) {
    GLint view[4];
    gl_get_integerv(GL_VIEWPORT, view);
    int w = view[2], h = view[3];
    unsigned char *pixels = malloc((size_t)w * h * 4);
    char path[4096];
    if (pixels_too) {
    // The engine binds its render targets for drawing only, so reads would still come from the
    // window: read from what the pass drew into.
    GLint draw = 0, read = 0;
    gl_get_integerv(0x8CA6 /* GL_DRAW_FRAMEBUFFER_BINDING */, &draw);
    gl_get_integerv(0x8CAA /* GL_READ_FRAMEBUFFER_BINDING */, &read);
    gl_bind_framebuffer(0x8CA8 /* GL_READ_FRAMEBUFFER */, draw);
    glPixelStorei(GL_PACK_ALIGNMENT, 1);
    gl_read_pixels(view[0], view[1], w, h, GL_RGBA, GL_UNSIGNED_BYTE, pixels);
    gl_bind_framebuffer(0x8CA8, read);
    snprintf(path, sizeof path, "%s/%s_%03d_%s.rgba", getenv("SHIM_DIR"), tag, frame, name);
    FILE *f = fopen(path, "wb");
    fwrite(pixels, 1, (size_t)w * h * 4, f);
    fclose(f);
    }
    free(pixels);
    snprintf(path, sizeof path, "%s/%s_%03d_%s.txt", getenv("SHIM_DIR"), tag, frame, name);
    FILE *f = fopen(path, "w");
    fprintf(f, "program %d fx %s\n", current_program, program_fx[current_program] ? program_fx[current_program] : "?");
    fprintf(f, "size %d %d\n", w, h);
    for (int l = 0; l < 64; l++)
        if (uniform_name[current_program][l])
            fprintf(f, "%s = %s\n", uniform_name[current_program][l], uniform_value[current_program][l]);
    fclose(f);
}

// Replaces the texture the NTSC pass reads as cleanFrameTexture with the input.
static void inject(void) {
    GLint loc = gl_get_uniform_location(current_program, "cleanFrameTexture"), unit = 0, tex = 0,
          active = 0, bound = 0;
    gl_get_uniformiv(current_program, loc, &unit);
    gl_get_integerv(GL_ACTIVE_TEXTURE, &active);
    gl_active_texture(GL_TEXTURE0 + unit);
    gl_get_integerv(GL_TEXTURE_BINDING_2D, &tex);
    glPixelStorei(GL_UNPACK_ALIGNMENT, 1);
    gl_tex_sub_image(GL_TEXTURE_2D, 0, 0, 0, 256, 224, GL_RGBA, GL_UNSIGNED_BYTE, input);
    gl_active_texture(active);
    (void)bound;
}

static int in_range_now(void) { return capturing && frame >= 0 && frame < frames_wanted; }
static void log_line(const char *text) {
    char path[4096];
    snprintf(path, sizeof path, "%s/%s_%03d_draws.txt", getenv("SHIM_DIR"), tag, frame);
    FILE *f = fopen(path, "a");
    if (f) { fputs(text, f); fclose(f); }
}
static void (*gl_generate_mipmap)(GLenum);
static void my_generate_mipmap(GLenum target) {
    if (in_range_now() && log_all) {
        GLint tex = 0, unit = 0; char t[256];
        gl_get_integerv(GL_TEXTURE_BINDING_2D, &tex);
        gl_get_integerv(GL_ACTIVE_TEXTURE, &unit);
        snprintf(t, sizeof t, "glGenerateMipmap target 0x%x texture %d unit %d (before draw %d)\n", target, tex, unit - GL_TEXTURE0, draw_no);
        log_line(t);
    }
    gl_generate_mipmap(target);
}
void glTexParameteri(GLenum target, GLenum pname, GLint param) {
    static void (*real)(GLenum, GLenum, GLint);
    if (!real) real = dlsym(RTLD_NEXT, "glTexParameteri");
    if (in_range_now() && log_all && gl_get_integerv) {
        GLint tex = 0; char t[256];
        gl_get_integerv(GL_TEXTURE_BINDING_2D, &tex);
        snprintf(t, sizeof t, "glTexParameteri texture %d pname 0x%x param 0x%x (before draw %d)\n", tex, pname, param, draw_no);
        log_line(t);
    }
    real(target, pname, param);
}
static void dump_prehud(void) {
    void (*get_tex_param)(GLenum, GLenum, GLint *) = libgl("glGetTexParameteriv");
    void (*get_level_param)(GLenum, GLint, GLenum, GLint *) = libgl("glGetTexLevelParameteriv");
    void (*get_tex_image)(GLenum, GLint, GLenum, GLenum, void *) = libgl("glGetTexImage");
    GLint loc = gl_get_uniform_location(current_program, "PreHUDBuffer"), unit = 0, active = 0, tex = 0;
    gl_get_uniformiv(current_program, loc, &unit);
    gl_get_integerv(GL_ACTIVE_TEXTURE, &active);
    gl_active_texture(GL_TEXTURE0 + unit);
    gl_get_integerv(GL_TEXTURE_BINDING_2D, &tex);
    char path[4096];
    snprintf(path, sizeof path, "%s/%s_%03d_prehud.txt", getenv("SHIM_DIR"), tag, frame);
    FILE *f = fopen(path, "w");
    if (!f) { gl_active_texture(active); return; }
    GLint v = 0;
    fprintf(f, "unit %d texture %d\n", unit, tex);
    get_tex_param(GL_TEXTURE_2D, GL_TEXTURE_MIN_FILTER, &v); fprintf(f, "MIN_FILTER 0x%x\n", v);
    get_tex_param(GL_TEXTURE_2D, GL_TEXTURE_MAG_FILTER, &v); fprintf(f, "MAG_FILTER 0x%x\n", v);
    get_tex_param(GL_TEXTURE_2D, 0x813C /* BASE_LEVEL */, &v); fprintf(f, "BASE_LEVEL %d\n", v);
    get_tex_param(GL_TEXTURE_2D, 0x813D /* MAX_LEVEL */, &v); fprintf(f, "MAX_LEVEL %d\n", v);
    for (int l = 0; l <= 11; l++) {
        GLint w = 0, h = 0, fmt = 0;
        get_level_param(GL_TEXTURE_2D, l, GL_TEXTURE_WIDTH, &w);
        get_level_param(GL_TEXTURE_2D, l, GL_TEXTURE_HEIGHT, &h);
        get_level_param(GL_TEXTURE_2D, l, GL_TEXTURE_INTERNAL_FORMAT, &fmt);
        fprintf(f, "level %d %dx%d internal 0x%x\n", l, w, h, fmt);
        if (w > 0 && h > 0 && l <= 6) {
            unsigned char *px = malloc((size_t)w * h * 4);
            glPixelStorei(GL_PACK_ALIGNMENT, 1);
            get_tex_image(GL_TEXTURE_2D, l, GL_RGBA, GL_UNSIGNED_BYTE, px);
            char p2[4096];
            snprintf(p2, sizeof p2, "%s/%s_%03d_prehud_L%d.rgba", getenv("SHIM_DIR"), tag, frame, l);
            FILE *g = fopen(p2, "wb");
            if (g) { fwrite(px, 1, (size_t)w * h * 4, g); fclose(g); }
            free(px);
        }
    }
    fclose(f);
    gl_active_texture(active);
}
static void log_draw(GLsizei count) {
    char path[4096];
    snprintf(path, sizeof path, "%s/%s_%03d_draws.txt", getenv("SHIM_DIR"), tag, frame);
    FILE *f = fopen(path, "a");
    if (!f) return;
    GLint view[4], draw = 0;
    gl_get_integerv(GL_VIEWPORT, view);
    gl_get_integerv(0x8CA6, &draw);
    int p = current_program > 0 && current_program < MAX_IDS ? current_program : 0;
    fprintf(f, "draw %d program %d fx %s kind %s fbo %d viewport %d %d %d %d indices %d\n", draw_no, p,
            p && program_fx[p] ? program_fx[p] : "?", pass_name[p ? program_kind[p] : 0], draw,
            view[0], view[1], view[2], view[3], (int)count);
    for (int l = 0; p && l < 64; l++)
        if (uniform_name[p][l]) fprintf(f, "    %s = %s\n", uniform_name[p][l], uniform_value[p][l]);
    fclose(f);
}
void glDrawElements(GLenum mode, GLsizei count, GLenum type, const void *indices) {
    if (!real_draw_elements) real_draw_elements = dlsym(RTLD_NEXT, "glDrawElements");
    int kind = current_program > 0 && current_program < MAX_IDS ? program_kind[current_program] : NONE;
    if (kind == NTSC) {
        check_go();
        if (injecting) inject();
    }
    int in_range = capturing && frame >= 0 && frame < frames_wanted;
    if (in_range && kind == SCREEN && clear_screen) {
        GLfloat old[4];
        glGetFloatv(GL_COLOR_CLEAR_VALUE, old);
        glClearColor(1, 0, 1, 1);
        glClear(GL_COLOR_BUFFER_BIT);
        glClearColor(old[0], old[1], old[2], old[3]);
    }
    if (in_range && kind == BLOOM_DOWN && mip_dump) dump_prehud();
    real_draw_elements(mode, count, type, indices);
    if (in_range && log_all) log_draw(count);
    if (in_range && kind != NONE) save(kind);
    else if (in_range && program_fx[current_program] && listed(readfx, program_fx[current_program])) {
        char name[256];
        snprintf(name, sizeof name, "d%03d_%s", draw_no, program_fx[current_program]);
        save_named(name, 1);
    }
    draw_no++;
    if (capturing && kind == COMPOSE) { frame++; draw_no = 0; }
}

// --- getting the functions -------------------------------------------------------------

struct wrap { const char *name; void *mine; void **real; };
static struct wrap wraps[] = {
    {"glShaderSource", my_shader_source, (void **)&gl_shader_source},
    {"glAttachShader", my_attach_shader, (void **)&gl_attach_shader},
    {"glLinkProgram", my_link_program, (void **)&gl_link_program},
    {"glUseProgram", my_use_program, (void **)&gl_use_program},
    {"glGetUniformLocation", my_get_uniform_location, (void **)&gl_get_uniform_location},
    {"glUniform1f", my_uniform1f, (void **)&gl_uniform1f},
    {"glUniform1i", my_uniform1i, (void **)&gl_uniform1i},
    {"glUniform1fv", my_uniform1fv, (void **)&gl_uniform1fv},
    {"glUniform2fv", my_uniform2fv, (void **)&gl_uniform2fv},
    {"glUniform3fv", my_uniform3fv, (void **)&gl_uniform3fv},
    {"glUniform4fv", my_uniform4fv, (void **)&gl_uniform4fv},
    {"glUniformMatrix4fv", my_uniform_matrix4fv, (void **)&gl_uniform_matrix4fv},
    {"glUniform2f", my_uniform2f, (void **)&gl_uniform2f},
    {"glUniform3f", my_uniform3f, (void **)&gl_uniform3f},
    {"glUniform4f", my_uniform4f, (void **)&gl_uniform4f},
    {"glGenerateMipmap", my_generate_mipmap, (void **)&gl_generate_mipmap},
};

static void resolve_helpers(void) {
    if (gl_read_pixels) return;
    gl_get_uniformiv = libgl("glGetUniformiv");
    gl_active_texture = libgl("glActiveTexture");
    gl_tex_sub_image = libgl("glTexSubImage2D");
    gl_read_pixels = libgl("glReadPixels");
    gl_get_integerv = libgl("glGetIntegerv");
    gl_bind_texture = libgl("glBindTexture");
    gl_bind_framebuffer = libgl("glBindFramebuffer");
}

void *glXGetProcAddressARB(const GLubyte *name) {
    if (!real_getproc) real_getproc = dlsym(RTLD_NEXT, "glXGetProcAddressARB");
    resolve_helpers();
    for (size_t i = 0; i < sizeof wraps / sizeof *wraps; i++) {
        size_t n = strlen(wraps[i].name);
        // The ARB names GLEW may ask for map onto the same wrappers.
        if (!strncmp((const char *)name, wraps[i].name, n) &&
            (name[n] == 0 || !strcmp((const char *)name + n, "ARB") || !strcmp((const char *)name + n, "EXT"))) {
            *wraps[i].real = real_getproc(name);
            if (!gl_get_uniform_location && !strcmp(wraps[i].name, "glGetUniformLocation"))
                gl_get_uniform_location = *wraps[i].real;
            return *wraps[i].real ? wraps[i].mine : NULL;
        }
    }
    return real_getproc(name);
}
void *glXGetProcAddress(const GLubyte *name) { return glXGetProcAddressARB(name); }
