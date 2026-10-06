// CC0. An LD_PRELOAD library for Super Win the Game's Linux build that feeds a test image into
// its CRT pipeline and reads back what each CRT pass draws, for comparing pass by pass.
//
// Build: gcc -m32 -shared -fPIC -O2 -o capture_shim.so capture_shim.c -ldl
// Run:   SHIM_DIR=out LD_PRELOAD=$PWD/capture_shim.so ./SuperGame_NFML
//
// The game draws with glDrawElements, which it imports, and loads GL 2.0 through
// glXGetProcAddressARB, which this wraps. Shader programs are told apart by uniforms only one
// CRT pass declares. Until SHIM_DIR/go exists the game runs untouched. The file holds lines:
//   input <path>   a 256x224 RGBA image, rows top first, raw bytes
//   frames <n>     frames to capture, counted in NTSC passes
//   skip <n>       frames to let pass first, so the glow of the scene before has decayed
//   tag <name>     prefix for the files written
// From then on, before every NTSC pass the Clean Frame texture is replaced with the input, and
// for the first n frames each CRT pass's target is read back to SHIM_DIR/<tag>_<frame>_<pass>
// .rgba (rows bottom first, as GL reads them) with its size and uniforms in a .txt beside it.
// Changing go's contents starts again.
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

static void my_shader_source(GLuint shader, GLsizei count, const GLchar_ *const *text,
                             const GLint *length) {
    int kind = NONE;
    for (int i = 0; i < count && shader < MAX_IDS; i++) {
        const char *t = text[i];
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
    return program_kind[current_program] ? uniform_value[current_program][loc] : NULL;
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
static int injecting, frames_wanted, frames_skipped, frame;
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
    for (; line; line = strtok(NULL, "\n")) {
        if (!strncmp(line, "input ", 6)) snprintf(image, sizeof image, "%s", line + 6);
        else if (!strncmp(line, "frames ", 7)) frames_wanted = atoi(line + 7);
        else if (!strncmp(line, "skip ", 5)) frames_skipped = atoi(line + 5);
        else if (!strncmp(line, "tag ", 4)) snprintf(tag, sizeof tag, "%s", line + 4);
    }
    FILE *img = fopen(image, "rb");
    if (!img) { fprintf(stderr, "shim: cannot open %s\n", image); return; }
    unsigned char *top_first = malloc(256 * 224 * 4);
    if (fread(top_first, 1, 256 * 224 * 4, img) != 256 * 224 * 4) fprintf(stderr, "shim: short image\n");
    fclose(img);
    free(input);
    input = malloc(256 * 224 * 4);
    for (int y = 0; y < 224; y++) memcpy(input + y * 1024, top_first + (223 - y) * 1024, 1024);
    free(top_first);
    frame = -frames_skipped;
    injecting = 1;
    fprintf(stderr, "shim: %s, %d frames from %s\n", tag, frames_wanted, image);
}

static void save(int kind) {
    GLint view[4];
    gl_get_integerv(GL_VIEWPORT, view);
    int w = view[2], h = view[3];
    unsigned char *pixels = malloc((size_t)w * h * 4);
    // The engine binds its render targets for drawing only, so reads would still come from the
    // window: read from what the pass drew into.
    GLint draw = 0, read = 0;
    gl_get_integerv(0x8CA6 /* GL_DRAW_FRAMEBUFFER_BINDING */, &draw);
    gl_get_integerv(0x8CAA /* GL_READ_FRAMEBUFFER_BINDING */, &read);
    gl_bind_framebuffer(0x8CA8 /* GL_READ_FRAMEBUFFER */, draw);
    glPixelStorei(GL_PACK_ALIGNMENT, 1);
    gl_read_pixels(view[0], view[1], w, h, GL_RGBA, GL_UNSIGNED_BYTE, pixels);
    gl_bind_framebuffer(0x8CA8, read);
    char path[4096];
    snprintf(path, sizeof path, "%s/%s_%03d_%s.rgba", getenv("SHIM_DIR"), tag, frame, pass_name[kind]);
    FILE *f = fopen(path, "wb");
    fwrite(pixels, 1, (size_t)w * h * 4, f);
    fclose(f);
    free(pixels);
    snprintf(path, sizeof path, "%s/%s_%03d_%s.txt", getenv("SHIM_DIR"), tag, frame, pass_name[kind]);
    f = fopen(path, "w");
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

void glDrawElements(GLenum mode, GLsizei count, GLenum type, const void *indices) {
    if (!real_draw_elements) real_draw_elements = dlsym(RTLD_NEXT, "glDrawElements");
    int kind = current_program > 0 && current_program < MAX_IDS ? program_kind[current_program] : NONE;
    if (kind == NTSC) {
        check_go();
        if (injecting) inject();
    }
    real_draw_elements(mode, count, type, indices);
    if (injecting && kind != NONE && frame >= 0 && frame < frames_wanted) save(kind);
    if (injecting && kind == COMPOSE) frame++;
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
            (name[n] == 0 || !strcmp((const char *)name + n, "ARB"))) {
            *wraps[i].real = real_getproc(name);
            if (!gl_get_uniform_location && !strcmp(wraps[i].name, "glGetUniformLocation"))
                gl_get_uniform_location = *wraps[i].real;
            return *wraps[i].real ? wraps[i].mine : NULL;
        }
    }
    return real_getproc(name);
}
void *glXGetProcAddress(const GLubyte *name) { return glXGetProcAddressARB(name); }
