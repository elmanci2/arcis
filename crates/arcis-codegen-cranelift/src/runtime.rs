//! C runtime that backs the Cranelift backend.
//!
//! The Arcis compiler's Cranelift backend has no access to the Rust standard
//! library. To keep binaries small and avoid a dependency on `rustc` or
//! `libstd`, we ship a tiny C runtime (`arcis_runtime.c`) implementing the
//! operations the lowered IR calls into: printing, string allocation,
//! number-to-string conversion, the platform entry stub (Linux `_start`).
//!
//! The single source-of-truth lives in [`RUNTIME_C_SOURCE`]: a `&'static str`
//! that the driver writes to a temp file and compiles with `cc -c` into
//! `arcis_runtime.o`, which is then linked together with the per-module
//! object files produced by the rest of this crate.
//!
//! ## ABI
//!
//! `ArcisString` values cross the runtime boundary as **opaque `int64_t`
//! handles** (a heap pointer to a `{ char*, uint64_t, uint64_t }` record).
//! Returning the struct by value would require Cranelift to know each
//! target's struct-by-value ABI; routing everything through a pointer
//! means the Cranelift backend only deals in `I64` parameters / returns,
//! which it knows how to handle.
//!
//! Ownership in Phase 1: alloc is owned by the program; `arcis_string_drop`
//! frees it. We intentionally do not yet insert drop calls — every
//! `ArcisString` is leaked until process exit. Memory pressure is fine:
//! the OS reclaims on exit. Proper drop insertion is Phase 2.

/// The C source text of the Arcis runtime. Embedded as a `&'static str` so
/// the compiler doesn't need to ship a separate `.c` file.
///
/// Functions exposed (all `extern "C"`):
/// - `ArcisString* arcis_string_from_cstr(const char*);`
/// - `ArcisString* arcis_string_alloc_bytes(const char*, uint64_t len);`
/// - `void arcis_string_drop(ArcisString*);`
/// - `void arcis_print(ArcisString*);`
/// - `void arcis_println(ArcisString*);`
/// - `ArcisString* arcis_num_to_string(double);`
/// - `ArcisString* arcis_string_concat(ArcisString*, ArcisString*);`
/// - `int32_t arcis_string_eq(ArcisString*, ArcisString*);`
/// - `int main(int argc, char** argv)` (entry stub — calls `arcis_main`)
pub const RUNTIME_C_SOURCE: &str = r#"
// arcis_runtime.c — Arcis runtime for the Cranelift backend.
//
// Compiled once by `arcis` via `cc -c arcis_runtime.c -o arcis_runtime.o`
// and linked with every program produced by the Cranelift backend.
//
// ABI: ArcisString is heap-allocated; values cross the FFI as int64_t
// pointers (handles). The runtime owns the heap and the handle encoding.

#include <stdint.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <ctype.h>
#include <unistd.h>
#include <dirent.h>
#include <sys/stat.h>
#include <sys/statvfs.h>
#include <sys/types.h>
#include <sys/wait.h>
#include <signal.h>
#include <errno.h>
#include <math.h>

typedef struct {
    char*    ptr;
    uint64_t len;
    uint64_t cap;
} ArcisString;

// ─────────────────────────────────────────────────────────────────────────────
// String allocation / deallocation
// ─────────────────────────────────────────────────────────────────────────────

static ArcisString* arcis_string_alloc_internal(const char* src, uint64_t len) {
    ArcisString* s = (ArcisString*)malloc(sizeof(ArcisString));
    if (s == NULL) {
        fputs("arcis: out of memory\n", stderr);
        abort();
    }
    s->cap = len + 16;
    s->len = len;
    s->ptr = (char*)malloc(s->cap);
    if (s->ptr == NULL) {
        fputs("arcis: out of memory\n", stderr);
        abort();
    }
    if (src != NULL && len > 0) {
        memcpy(s->ptr, src, len);
    }
    s->ptr[len] = '\0';
    return s;
}

ArcisString* arcis_string_from_cstr(const char* s) {
    if (s == NULL) {
        return arcis_string_alloc_internal(NULL, 0);
    }
    return arcis_string_alloc_internal(s, strlen(s));
}

ArcisString* arcis_string_alloc_bytes(const char* src, uint64_t len) {
    return arcis_string_alloc_internal(src, len);
}

void arcis_string_drop(ArcisString* s) {
    if (s == NULL) {
        return;
    }
    if (s->ptr != NULL) {
        free(s->ptr);
    }
    free(s);
}

// ─────────────────────────────────────────────────────────────────────────────
// Print (Phase 1)
// ─────────────────────────────────────────────────────────────────────────────

void arcis_print(ArcisString* s) {
    if (s != NULL && s->len > 0) {
        fwrite(s->ptr, 1, s->len, stdout);
    }
}

void arcis_println(ArcisString* s) {
    arcis_print(s);
    fputc('\n', stdout);
    fflush(stdout);
}

// ─────────────────────────────────────────────────────────────────────────────
// Number → string (Phase 1 — needed so we can `print(42)`)
// ─────────────────────────────────────────────────────────────────────────────

ArcisString* arcis_num_to_string(double v) {
    // Mirrors Rust's f64::Display: pick the shortest decimal that
    // round-trips back to the same f64. We try precision levels 1..17
    // and pick the shortest match that round-trips AND does not require
    // scientific notation (matches Rust, which avoids 'e+...' for
    // ordinary-size numbers). Standard C doesn't ship Ryu, so we emulate
    // it with strtod round-trip checks — slow but correct.
    static const int precisions[] = {1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13, 14, 15, 16, 17};
    char buf[64];
    for (size_t i = 0; i < sizeof(precisions) / sizeof(precisions[0]); i++) {
        int len = snprintf(buf, sizeof(buf), "%.*g", precisions[i], (double)v);
        if (len < 0) {
            return arcis_string_alloc_internal(NULL, 0);
        }
        // Reject scientific notation unless the value is tiny/huge and
        // there is no other option (matches Rust Display behaviour).
        if (strchr(buf, 'e') != NULL || strchr(buf, 'E') != NULL) {
            continue;
        }
        char* end = NULL;
        double parsed = strtod(buf, &end);
        if (end != buf && parsed == (double)v) {
            return arcis_string_alloc_internal(buf, (uint64_t)len);
        }
    }
    // Fall back to scientific notation for values that genuinely need it
    // (very large / very small magnitudes).
    int len = snprintf(buf, sizeof(buf), "%.17g", (double)v);
    return arcis_string_alloc_internal(buf, (uint64_t)(len < 0 ? 0 : (uint64_t)len));
}

ArcisString* arcis_bool_to_string(int32_t v) {
    if (v) {
        return arcis_string_alloc_internal("true", 4);
    }
    return arcis_string_alloc_internal("false", 5);
}

// ─────────────────────────────────────────────────────────────────────────────
// String concat + equality (Phase 1 — needed for `print("x=" + n)` style)
// ─────────────────────────────────────────────────────────────────────────────

ArcisString* arcis_string_concat(ArcisString* a, ArcisString* b) {
    if (a == NULL) {
        a = arcis_string_alloc_internal(NULL, 0);
    }
    if (b == NULL) {
        b = arcis_string_alloc_internal(NULL, 0);
    }
    uint64_t total = a->len + b->len;
    char* buf = (char*)malloc(total + 16);
    if (buf == NULL) {
        fputs("arcis: out of memory\n", stderr);
        abort();
    }
    memcpy(buf, a->ptr, a->len);
    memcpy(buf + a->len, b->ptr, b->len);
    buf[total] = '\0';
    ArcisString* out = (ArcisString*)malloc(sizeof(ArcisString));
    if (out == NULL) {
        fputs("arcis: out of memory\n", stderr);
        abort();
    }
    out->ptr = buf;
    out->len = total;
    out->cap = total + 16;
    return out;
}

int32_t arcis_string_eq(ArcisString* a, ArcisString* b) {
    if (a == NULL || b == NULL) {
        return a == b ? 1 : 0;
    }
    if (a->len != b->len) {
        return 0;
    }
    return memcmp(a->ptr, b->ptr, a->len) == 0 ? 1 : 0;
}

// ─────────────────────────────────────────────────────────────────────────────
// Process id (placeholder; Phase 1 only needs printing for hello-world)
// ─────────────────────────────────────────────────────────────────────────────

int64_t arcis_current_pid(void) {
    return (int64_t)getpid();
}

// ─────────────────────────────────────────────────────────────────────────────
// parseFloat (Phase 4)
// ─────────────────────────────────────────────────────────────────────────────

double arcis_parse_float(ArcisString* s) {
    if (s == NULL || s->len == 0) return NAN;
    char tmp[4096];
    snprintf(tmp, sizeof(tmp), "%.*s", (int)s->len, s->ptr);
    char* end = NULL;
    double val = strtod(tmp, &end);
    // Return NaN if no digits parsed or trailing garbage remains.
    if (end == tmp || *end != '\0') return NAN;
    return val;
}

int32_t arcis_is_nan(double v) {
    return isnan(v) ? 1 : 0;
}

// ─────────────────────────────────────────────────────────────────────────────
// String methods (Phase 2)
// ─────────────────────────────────────────────────────────────────────────────

ArcisString* arcis_string_to_uppercase(ArcisString* s) {
    if (s == NULL || s->len == 0) {
        return arcis_string_alloc_internal(NULL, 0);
    }
    ArcisString* out = arcis_string_alloc_internal(NULL, s->len);
    if (out == NULL) return arcis_string_alloc_internal(NULL, 0);
    for (uint64_t i = 0; i < s->len; i++) {
        out->ptr[i] = (char)toupper((unsigned char)s->ptr[i]);
    }
    out->ptr[s->len] = '\0';
    out->len = s->len;
    return out;
}

ArcisString* arcis_string_to_lowercase(ArcisString* s) {
    if (s == NULL || s->len == 0) {
        return arcis_string_alloc_internal(NULL, 0);
    }
    ArcisString* out = arcis_string_alloc_internal(NULL, s->len);
    if (out == NULL) return arcis_string_alloc_internal(NULL, 0);
    for (uint64_t i = 0; i < s->len; i++) {
        out->ptr[i] = (char)tolower((unsigned char)s->ptr[i]);
    }
    out->ptr[s->len] = '\0';
    out->len = s->len;
    return out;
}

ArcisString* arcis_string_trim(ArcisString* s) {
    if (s == NULL || s->len == 0) {
        return arcis_string_alloc_internal(NULL, 0);
    }
    uint64_t start = 0;
    while (start < s->len && isspace((unsigned char)s->ptr[start])) {
        start++;
    }
    uint64_t end = s->len;
    while (end > start && isspace((unsigned char)s->ptr[end - 1])) {
        end--;
    }
    uint64_t new_len = end - start;
    return arcis_string_alloc_internal(s->ptr + start, new_len);
}

ArcisString* arcis_string_substring(ArcisString* s, int64_t a, int64_t b) {
    if (s == NULL || s->len == 0) {
        return arcis_string_alloc_internal(NULL, 0);
    }
    uint64_t start = (a < 0) ? 0 : (uint64_t)a;
    uint64_t end   = (b < 0) ? 0 : (uint64_t)b;
    if (start > s->len) start = s->len;
    if (end > s->len) end = s->len;
    if (start > end) start = end;
    uint64_t new_len = end - start;
    return arcis_string_alloc_internal(s->ptr + start, new_len);
}

double arcis_string_index_of(ArcisString* haystack, ArcisString* needle) {
    if (haystack == NULL || needle == NULL) return -1.0;
    if (needle->len == 0) return 0.0;
    if (needle->len > haystack->len) return -1.0;
    for (uint64_t i = 0; i <= haystack->len - needle->len; i++) {
        if (memcmp(haystack->ptr + i, needle->ptr, needle->len) == 0) {
            return (double)i;
        }
    }
    return -1.0;
}

int32_t arcis_string_includes(ArcisString* haystack, ArcisString* needle) {
    double idx = arcis_string_index_of(haystack, needle);
    return idx >= 0.0 ? 1 : 0;
}

ArcisString* arcis_string_char_at(ArcisString* s, int64_t idx) {
    if (s == NULL || idx < 0 || (uint64_t)idx >= s->len) {
        return arcis_string_alloc_internal(NULL, 0);
    }
    return arcis_string_alloc_internal(s->ptr + (uint64_t)idx, 1);
}

// ─────────────────────────────────────────────────────────────────────────────
// Input (Phase 2)
// ─────────────────────────────────────────────────────────────────────────────

ArcisString* arcis_read_line(void) {
    char* line = NULL;
    size_t cap = 0;
    ssize_t n = getline(&line, &cap, stdin);
    if (n < 0) {
        if (line != NULL) free(line);
        return arcis_string_alloc_internal(NULL, 0);
    }
    // Drop trailing newline if present.
    if (n > 0 && line[n - 1] == '\n') {
        n--;
    }
    ArcisString* out = arcis_string_alloc_internal(line, (uint64_t)n);
    free(line);
    return out;
}

// ─────────────────────────────────────────────────────────────────────────────
// Arrays: ArcisVec (Phase 2)
//
// ArcisVec is a growable array of `int64_t` slots. Every Arcis value
// (number, string handle, boolean, object handle) is stored as a single
// `int64_t`.  The Cranelift codegen stores `f64` numbers as their raw
// bits inside the slot and converts with `bitcast` when loading/storing.
// ─────────────────────────────────────────────────────────────────────────────

typedef struct {
    int64_t* elements;
    int32_t  len;
    int32_t  cap;
} ArcisVec;

ArcisVec* arcis_vec_new(void) {
    ArcisVec* v = (ArcisVec*)malloc(sizeof(ArcisVec));
    if (v == NULL) abort();
    v->elements = NULL;
    v->len = 0;
    v->cap = 0;
    return v;
}

void arcis_vec_push(ArcisVec* v, int64_t elem) {
    if (v == NULL) return;
    if (v->len >= v->cap) {
        int32_t new_cap = v->cap == 0 ? 8 : v->cap * 2;
        int64_t* p = (int64_t*)realloc(v->elements, (size_t)new_cap * sizeof(int64_t));
        if (p == NULL) abort();
        v->elements = p;
        v->cap = new_cap;
    }
    v->elements[v->len++] = elem;
}

int64_t arcis_vec_pop(ArcisVec* v) {
    if (v == NULL || v->len == 0) return 0;
    return v->elements[--v->len];
}

void arcis_vec_unshift(ArcisVec* v, int64_t elem) {
    if (v == NULL) return;
    if (v->len >= v->cap) {
        int32_t new_cap = v->cap == 0 ? 8 : v->cap * 2;
        int64_t* p = (int64_t*)realloc(v->elements, (size_t)new_cap * sizeof(int64_t));
        if (p == NULL) abort();
        v->elements = p;
        v->cap = new_cap;
    }
    // Memmove right by one.
    if (v->len > 0) {
        memmove(v->elements + 1, v->elements, (size_t)v->len * sizeof(int64_t));
    }
    v->elements[0] = elem;
    v->len++;
}

int32_t arcis_vec_len(ArcisVec* v) {
    if (v == NULL) return 0;
    return v->len;
}

int64_t arcis_vec_get(ArcisVec* v, int32_t idx) {
    if (v == NULL || idx < 0 || idx >= v->len) return 0;
    return v->elements[idx];
}

void arcis_vec_set(ArcisVec* v, int32_t idx, int64_t elem) {
    if (v == NULL || idx < 0 || idx >= v->len) return;
    v->elements[idx] = elem;
}

void arcis_vec_drop(ArcisVec* v) {
    if (v == NULL) return;
    if (v->elements != NULL) free(v->elements);
    free(v);
}

// ─────────────────────────────────────────────────────────────────────────────
// Objects: ArcisObject (Phase 2 stub — dictionary-based)
//
// Phase 2 stores object fields as key-value pairs. Phase 3 will add
// field-specific get/set with typed struct layout when object shapes
// are known at compile time.
// ─────────────────────────────────────────────────────────────────────────────

typedef struct {
    ArcisString* key_handle;
    int64_t      value;
} ArcisField;

typedef struct {
    ArcisField* fields;
    int32_t     count;
    int32_t     cap;
} ArcisObject;

ArcisObject* arcis_object_new(void) {
    ArcisObject* obj = (ArcisObject*)malloc(sizeof(ArcisObject));
    if (obj == NULL) abort();
    obj->fields = NULL;
    obj->count = 0;
    obj->cap = 0;
    return obj;
}

void arcis_object_set(ArcisObject* obj, ArcisString* key, int64_t value) {
    if (obj == NULL || key == NULL) return;
    for (int32_t i = 0; i < obj->count; i++) {
        ArcisField* f = &obj->fields[i];
        if (f->key_handle != NULL) {
            ArcisString* ek = f->key_handle;
            if (ek->len == key->len && memcmp(ek->ptr, key->ptr, key->len) == 0) {
                f->value = value;
                return;
            }
        }
    }
    if (obj->count >= obj->cap) {
        int32_t new_cap = obj->cap == 0 ? 4 : obj->cap * 2;
        ArcisField* p = (ArcisField*)realloc(obj->fields, (size_t)new_cap * sizeof(ArcisField));
        if (p == NULL) abort();
        obj->fields = p;
        obj->cap = new_cap;
    }
    // Clone the key handle so it survives the caller.
    ArcisString* cloned = arcis_string_alloc_internal(key->ptr, key->len);
    obj->fields[obj->count].key_handle = cloned;
    obj->fields[obj->count].value = value;
    obj->count++;
}

int64_t arcis_object_get(ArcisObject* obj, ArcisString* key) {
    if (obj == NULL || key == NULL || key->len == 0) return 0;
    for (int32_t i = 0; i < obj->count; i++) {
        ArcisString* existing = obj->fields[i].key_handle;
        if (existing != NULL && existing->len == key->len
            && memcmp(existing->ptr, key->ptr, key->len) == 0) {
            return obj->fields[i].value;
        }
    }
    return 0;
}

void arcis_object_drop(ArcisObject* obj) {
    if (obj == NULL) return;
    // Keys are ArcisString* handles owned by the object; free them.
    for (int32_t i = 0; i < obj->count; i++) {
        arcis_string_drop(obj->fields[i].key_handle);
    }
    if (obj->fields != NULL) free(obj->fields);
    free(obj);
}

// ─────────────────────────────────────────────────────────────────────────────
// ArcisProcess placeholder struct (Phase 5)
// ─────────────────────────────────────────────────────────────────────────────

typedef struct {
    ArcisString* stdout_;
    ArcisString* stderr_;
    int32_t      exit_code;
} ArcisProcess;

// ─────────────────────────────────────────────────────────────────────────────
// sys.* — filesystem (Phase 5)
// ─────────────────────────────────────────────────────────────────────────────

ArcisString* arcis_fs_read_file(ArcisString* path) {
    if (path == NULL || path->len == 0) return arcis_string_alloc_internal(NULL, 0);
    char tmp[4096]; snprintf(tmp, sizeof(tmp), "%.*s", (int)path->len, path->ptr);
    FILE* f = fopen(tmp, "rb");
    if (f == NULL) return arcis_string_alloc_internal(NULL, 0);
    fseek(f, 0, SEEK_END); long sz = ftell(f); fseek(f, 0, SEEK_SET);
    ArcisString* out = arcis_string_alloc_internal(NULL, (uint64_t)(sz > 0 ? sz : 0));
    if (sz > 0 && out != NULL) fread(out->ptr, 1, (size_t)sz, f);
    fclose(f);
    return out;
}

void arcis_fs_write_file(ArcisString* path, ArcisString* content) {
    if (path == NULL || path->len == 0 || content == NULL) return;
    char tmp[4096]; snprintf(tmp, sizeof(tmp), "%.*s", (int)path->len, path->ptr);
    FILE* f = fopen(tmp, "wb");
    if (f == NULL) return;
    if (content->len > 0) fwrite(content->ptr, 1, content->len, f);
    fclose(f);
}

ArcisString* arcis_fs_read_bytes(ArcisString* path) { return arcis_fs_read_file(path); }
void arcis_fs_write_bytes(ArcisString* path, ArcisString* bytes) { arcis_fs_write_file(path, bytes); }

void arcis_fs_append_file(ArcisString* path, ArcisString* text) {
    if (path == NULL || path->len == 0 || text == NULL) return;
    char tmp[4096]; snprintf(tmp, sizeof(tmp), "%.*s", (int)path->len, path->ptr);
    FILE* f = fopen(tmp, "ab");
    if (f == NULL) return;
    if (text->len > 0) fwrite(text->ptr, 1, text->len, f);
    fclose(f);
}

void arcis_fs_create_file(ArcisString* path) {
    if (path == NULL || path->len == 0) return;
    char tmp[4096]; snprintf(tmp, sizeof(tmp), "%.*s", (int)path->len, path->ptr);
    FILE* f = fopen(tmp, "w"); if (f) fclose(f);
}

void arcis_fs_delete_file(ArcisString* path) {
    if (path == NULL || path->len == 0) return;
    char tmp[4096]; snprintf(tmp, sizeof(tmp), "%.*s", (int)path->len, path->ptr);
    remove(tmp);
}

void arcis_fs_delete_dir(ArcisString* path) {
    if (path == NULL || path->len == 0) return;
    char tmp[4096]; snprintf(tmp, sizeof(tmp), "%.*s", (int)path->len, path->ptr);
    rmdir(tmp);
}

void arcis_fs_delete_dir_all(ArcisString* path) {
    // Use system rm -rf for simplicity.
    if (path == NULL || path->len == 0) return;
    char cmd[8192];
    snprintf(cmd, sizeof(cmd), "rm -rf '%.*s'", (int)path->len, path->ptr);
    system(cmd);
}

void arcis_fs_mkdir(ArcisString* path) {
    if (path == NULL || path->len == 0) return;
    char tmp[4096]; snprintf(tmp, sizeof(tmp), "%.*s", (int)path->len, path->ptr);
    mkdir(tmp, 0755);
}

ArcisVec* arcis_fs_list_dir(ArcisString* path) {
    ArcisVec* v = arcis_vec_new();
    if (path == NULL || path->len == 0) return v;
    char tmp[4096]; snprintf(tmp, sizeof(tmp), "%.*s", (int)path->len, path->ptr);
    DIR* d = opendir(tmp);
    if (d == NULL) return v;
    struct dirent* ent;
    while ((ent = readdir(d)) != NULL) {
        if (strcmp(ent->d_name, ".") == 0 || strcmp(ent->d_name, "..") == 0) continue;
        ArcisString* name = arcis_string_from_cstr(ent->d_name);
        arcis_vec_push(v, (int64_t)(uintptr_t)name);
    }
    closedir(d);
    return v;
}

void arcis_fs_copy(ArcisString* src, ArcisString* dst) {
    if (src == NULL || dst == NULL) return;
    char s[4096], d[4096];
    snprintf(s, sizeof(s), "%.*s", (int)src->len, src->ptr);
    snprintf(d, sizeof(d), "%.*s", (int)dst->len, dst->ptr);
    FILE* fs = fopen(s, "rb"); if (fs == NULL) return;
    FILE* fd = fopen(d, "wb"); if (fd == NULL) { fclose(fs); return; }
    char buf[8192]; size_t n;
    while ((n = fread(buf, 1, sizeof(buf), fs)) > 0) fwrite(buf, 1, n, fd);
    fclose(fs); fclose(fd);
}

void arcis_fs_move(ArcisString* src, ArcisString* dst) {
    if (src == NULL || dst == NULL) return;
    char s[4096], d[4096];
    snprintf(s, sizeof(s), "%.*s", (int)src->len, src->ptr);
    snprintf(d, sizeof(d), "%.*s", (int)dst->len, dst->ptr);
    rename(s, d);
}

// ─────────────────────────────────────────────────────────────────────────────
// sys.* — path queries (Phase 5)
// ─────────────────────────────────────────────────────────────────────────────

double arcis_path_exists(ArcisString* p) {
    if (p == NULL || p->len == 0) return 0.0;
    char tmp[4096]; snprintf(tmp, sizeof(tmp), "%.*s", (int)p->len, p->ptr);
    return access(tmp, F_OK) == 0 ? 1.0 : 0.0;
}

static double arcis_path_is_type(ArcisString* p, int mode) {
    if (p == NULL || p->len == 0) return 0.0;
    char tmp[4096]; snprintf(tmp, sizeof(tmp), "%.*s", (int)p->len, p->ptr);
    struct stat st;
    if (stat(tmp, &st) != 0) return 0.0;
    if (mode == 0) return S_ISREG(st.st_mode) ? 1.0 : 0.0;
    return S_ISDIR(st.st_mode) ? 1.0 : 0.0;
}
double arcis_path_is_file(ArcisString* p) { return arcis_path_is_type(p, 0); }
double arcis_path_is_dir(ArcisString* p) { return arcis_path_is_type(p, 1); }

double arcis_path_file_size(ArcisString* p) {
    if (p == NULL || p->len == 0) return 0.0;
    char tmp[4096]; snprintf(tmp, sizeof(tmp), "%.*s", (int)p->len, p->ptr);
    struct stat st;
    if (stat(tmp, &st) != 0) return 0.0;
    return (double)st.st_size;
}

ArcisString* arcis_path_absolute(ArcisString* p) {
    if (p == NULL || p->len == 0) return arcis_string_alloc_internal(NULL, 0);
    char tmp[4096]; snprintf(tmp, sizeof(tmp), "%.*s", (int)p->len, p->ptr);
    char* resolved = realpath(tmp, NULL);
    if (resolved == NULL) return arcis_string_alloc_internal(tmp, (uint64_t)strlen(tmp));
    ArcisString* out = arcis_string_from_cstr(resolved);
    free(resolved);
    return out;
}

ArcisString* arcis_path_file_info(ArcisString* p) {
    if (p == NULL || p->len == 0) return arcis_string_alloc_internal(NULL, 0);
    char tmp[4096]; snprintf(tmp, sizeof(tmp), "%.*s", (int)p->len, p->ptr);
    struct stat st;
    if (stat(tmp, &st) != 0) return arcis_string_from_cstr("");
    char buf[512];
    snprintf(buf, sizeof(buf), "size=%ld;is_file=%d;is_dir=%d;modified_secs=%ld",
        (long)st.st_size, S_ISREG(st.st_mode) ? 1 : 0, S_ISDIR(st.st_mode) ? 1 : 0, (long)st.st_mtime);
    return arcis_string_from_cstr(buf);
}

// ─────────────────────────────────────────────────────────────────────────────
// sys.* — process env (Phase 5)
// ─────────────────────────────────────────────────────────────────────────────

ArcisString* arcis_proc_current_dir(void) {
    char buf[4096];
    if (getcwd(buf, sizeof(buf)) == NULL) return arcis_string_alloc_internal(NULL, 0);
    return arcis_string_from_cstr(buf);
}
void arcis_proc_change_dir(ArcisString* p) {
    if (p == NULL) return;
    char tmp[4096]; snprintf(tmp, sizeof(tmp), "%.*s", (int)p->len, p->ptr);
    chdir(tmp);
}
ArcisString* arcis_proc_temp_dir(void) {
    const char* t = getenv("TMPDIR");
    if (t == NULL) t = "/tmp";
    return arcis_string_from_cstr(t);
}
ArcisString* arcis_proc_home_dir(void) {
    const char* h = getenv("HOME");
    if (h == NULL) h = "";
    return arcis_string_from_cstr(h);
}
ArcisString* arcis_proc_executable_path(void) {
    char buf[4096];
    ssize_t n = readlink("/proc/self/exe", buf, sizeof(buf) - 1);
    if (n < 0) return arcis_string_alloc_internal(NULL, 0);
    buf[n] = '\0';
    return arcis_string_from_cstr(buf);
}

// ─────────────────────────────────────────────────────────────────────────────
// sys.* — process management (Phase 5)
// ─────────────────────────────────────────────────────────────────────────────

double arcis_process_current_pid(void) { return (double)getpid(); }
double arcis_process_parent_pid(void) { return (double)getppid(); }

ArcisString* arcis_process_exec(ArcisString* cmd, ArcisVec* args_v) {
    // Build argv, run cmd, capture stdout, return as string.
    if (cmd == NULL || cmd->len == 0) return arcis_string_alloc_internal(NULL, 0);
    char c[4096]; snprintf(c, sizeof(c), "%.*s", (int)cmd->len, cmd->ptr);
    FILE* p = popen(c, "r");
    if (p == NULL) return arcis_string_alloc_internal(NULL, 0);
    ArcisString* out = arcis_string_alloc_internal(NULL, 0);
    char buf[4096];
    size_t n;
    while ((n = fread(buf, 1, sizeof(buf), p)) > 0) {
        ArcisString* chunk = arcis_string_alloc_internal(buf, (uint64_t)n);
        ArcisString* tmp = arcis_string_concat(out, chunk);
        arcis_string_drop(out); arcis_string_drop(chunk);
        out = tmp;
    }
    pclose(p);
    return out;
}

ArcisString* arcis_process_run(ArcisString* cmd) {
    return arcis_process_exec(cmd, NULL);
}

double arcis_process_spawn(ArcisString* cmd) {
    if (cmd == NULL || cmd->len == 0) return -1.0;
    char c[4096]; snprintf(c, sizeof(c), "%.*s", (int)cmd->len, cmd->ptr);
    pid_t pid = fork();
    if (pid == 0) { execl("/bin/sh", "sh", "-c", c, (char*)NULL); _exit(127); }
    return (double)pid;
}

void arcis_process_kill(double pid) {
    if (pid <= 0) return;
    kill((pid_t)pid, SIGTERM);
}

ArcisVec* arcis_process_list(void) {
    ArcisVec* v = arcis_vec_new();
    FILE* p = popen("ps -e -o pid=,comm=", "r");
    if (p == NULL) return v;
    char buf[512];
    while (fgets(buf, sizeof(buf), p) != NULL) {
        char* nl = strchr(buf, '\n'); if (nl) *nl = '\0';
        ArcisString* s = arcis_string_from_cstr(buf);
        arcis_vec_push(v, (int64_t)(uintptr_t)s);
    }
    pclose(p);
    return v;
}

// ─────────────────────────────────────────────────────────────────────────────
// sys.* — env vars (Phase 5)
// ─────────────────────────────────────────────────────────────────────────────

ArcisString* arcis_env_get(ArcisString* name) {
    if (name == NULL || name->len == 0) return arcis_string_alloc_internal(NULL, 0);
    char tmp[4096]; snprintf(tmp, sizeof(tmp), "%.*s", (int)name->len, name->ptr);
    const char* v = getenv(tmp);
    return arcis_string_from_cstr(v != NULL ? v : "");
}
void arcis_env_set(ArcisString* name, ArcisString* value) {
    if (name == NULL || name->len == 0) return;
    char n[4096], v[4096];
    snprintf(n, sizeof(n), "%.*s", (int)name->len, name->ptr);
    snprintf(v, sizeof(v), "%.*s", value != NULL ? (int)value->len : 0, value != NULL ? value->ptr : "");
    setenv(n, v, 1);
}
void arcis_env_delete(ArcisString* name) {
    if (name == NULL || name->len == 0) return;
    char tmp[4096]; snprintf(tmp, sizeof(tmp), "%.*s", (int)name->len, name->ptr);
    unsetenv(tmp);
}
ArcisVec* arcis_env_all(void) {
    ArcisVec* v = arcis_vec_new();
    extern char** environ;
    for (char** e = environ; *e != NULL; e++) {
        ArcisString* s = arcis_string_from_cstr(*e);
        arcis_vec_push(v, (int64_t)(uintptr_t)s);
    }
    return v;
}

// ─────────────────────────────────────────────────────────────────────────────
// sys.* — OS info (Phase 5)
// ─────────────────────────────────────────────────────────────────────────────

ArcisString* arcis_os_name(void) {
#ifdef __linux__
    return arcis_string_from_cstr("linux");
#elif defined(__APPLE__)
    return arcis_string_from_cstr("macos");
#else
    return arcis_string_from_cstr("unknown");
#endif
}

ArcisString* arcis_os_arch(void) {
#if defined(__x86_64__) || defined(_M_X64)
    return arcis_string_from_cstr("x86_64");
#elif defined(__aarch64__)
    return arcis_string_from_cstr("aarch64");
#else
    return arcis_string_from_cstr("unknown");
#endif
}

ArcisString* arcis_os_version(void) { return arcis_process_exec(arcis_string_from_cstr("uname -r"), NULL); }
ArcisString* arcis_os_hostname(void) { return arcis_process_exec(arcis_string_from_cstr("hostname"), NULL); }
ArcisString* arcis_os_username(void) {
    const char* u = getenv("USER");
    if (u == NULL) u = getenv("USERNAME");
    return arcis_string_from_cstr(u != NULL ? u : "");
}
double arcis_os_uptime(void) {
    FILE* f = fopen("/proc/uptime", "r");
    if (f == NULL) return 0.0;
    double up = 0.0; fscanf(f, "%lf", &up); fclose(f);
    return up;
}
ArcisString* arcis_os_locale(void) {
    const char* l = getenv("LC_ALL");
    if (l == NULL) l = getenv("LANG");
    return arcis_string_from_cstr(l != NULL ? l : "");
}
double arcis_os_cpu_count(void) {
    long n = sysconf(_SC_NPROCESSORS_ONLN);
    return n > 0 ? (double)n : 1.0;
}

// ─────────────────────────────────────────────────────────────────────────────
// sys.* — memory (Phase 5) — Linux-specific
// ─────────────────────────────────────────────────────────────────────────────

static double arcis_meminfo_kb(const char* field) {
    FILE* f = fopen("/proc/meminfo", "r");
    if (f == NULL) return 0.0;
    char line[256]; double kb = 0.0;
    while (fgets(line, sizeof(line), f) != NULL) {
        if (strncmp(line, field, strlen(field)) == 0) {
            sscanf(line + strlen(field), " : %lf", &kb);
            break;
        }
    }
    fclose(f);
    return kb * 1024.0;
}
double arcis_memory_total(void)     { return arcis_meminfo_kb("MemTotal"); }
double arcis_memory_free(void)      { return arcis_meminfo_kb("MemFree"); }
double arcis_memory_used(void)      { return arcis_meminfo_kb("MemTotal") - arcis_meminfo_kb("MemFree"); }
double arcis_memory_available(void) { return arcis_meminfo_kb("MemAvailable"); }

// ─────────────────────────────────────────────────────────────────────────────
// sys.* — CPU (Phase 5) — Linux-specific
// ─────────────────────────────────────────────────────────────────────────────

static ArcisString* arcis_cpuinfo_field(const char* field) {
    FILE* f = fopen("/proc/cpuinfo", "r");
    if (f == NULL) return arcis_string_from_cstr("");
    char line[512];
    while (fgets(line, sizeof(line), f) != NULL) {
        if (strncmp(line, field, strlen(field)) == 0) {
            char* colon = strchr(line, ':');
            if (colon != NULL) {
                char* val = colon + 1;
                while (*val == ' ' || *val == '\t') val++;
                size_t len = strlen(val);
                while (len > 0 && (val[len-1] == '\n' || val[len-1] == '\r')) len--;
                fclose(f);
                ArcisString* out = arcis_string_alloc_internal(NULL, (uint64_t)len);
                if (out != NULL) memcpy(out->ptr, val, len);
                return out;
            }
        }
    }
    fclose(f);
    return arcis_string_from_cstr("");
}
ArcisString* arcis_cpu_model(void)     { return arcis_cpuinfo_field("model name"); }
ArcisString* arcis_cpu_brand(void)     { return arcis_cpuinfo_field("vendor_id"); }
double arcis_cpu_frequency(void) {
    FILE* f = fopen("/proc/cpuinfo", "r");
    if (f == NULL) return 0.0;
    char line[256]; double mhz = 0.0;
    while (fgets(line, sizeof(line), f) != NULL) {
        if (strncmp(line, "cpu MHz", 7) == 0) {
            char* colon = strchr(line, ':');
            if (colon != NULL) { mhz = atof(colon + 1); break; }
        }
    }
    fclose(f);
    return mhz;
}
double arcis_cpu_usage(void) {
    // Read /proc/stat, sleep 100ms, read again, compute busy %.
    FILE* f = fopen("/proc/stat", "r");
    if (f == NULL) return 0.0;
    char line[512]; double u1 = 0, n1 = 0, u2 = 0, n2 = 0;
    if (fgets(line, sizeof(line), f) && strncmp(line, "cpu ", 4) == 0) {
        double a[10] = {0}; int i;
        sscanf(line + 4, "%lf %lf %lf %lf %lf %lf %lf %lf %lf %lf",
            &a[0],&a[1],&a[2],&a[3],&a[4],&a[5],&a[6],&a[7],&a[8],&a[9]);
        for (i = 0; i < 10; i++) { u1 += a[i]; if (i == 3) n1 = a[3]; }
    }
    fclose(f);
    usleep(100000);
    f = fopen("/proc/stat", "r");
    if (f == NULL) return 0.0;
    if (fgets(line, sizeof(line), f) && strncmp(line, "cpu ", 4) == 0) {
        double a[10] = {0}; int i;
        sscanf(line + 4, "%lf %lf %lf %lf %lf %lf %lf %lf %lf %lf",
            &a[0],&a[1],&a[2],&a[3],&a[4],&a[5],&a[6],&a[7],&a[8],&a[9]);
        for (i = 0; i < 10; i++) { u2 += a[i]; if (i == 3) n2 = a[3]; }
    }
    fclose(f);
    double dtotal = u2 - u1, didle = n2 - n1;
    if (dtotal <= 0.0) return 0.0;
    return ((dtotal - didle) / dtotal) * 100.0;
}
double arcis_cpu_cores(void) { return arcis_os_cpu_count(); }

// ─────────────────────────────────────────────────────────────────────────────
// sys.* — GPU, Disk, Net (Phase 5) — via subprocess
// ─────────────────────────────────────────────────────────────────────────────

ArcisVec* arcis_gpu_list(void) {
    ArcisString* cmd = arcis_string_from_cstr("lspci -vmm 2>/dev/null | grep -A2 'VGA\\|3D\\|Display'");
    ArcisString* out = arcis_process_exec(cmd, NULL);
    // Parse into vendor=...;name=... format. For simplicity return raw.
    ArcisVec* v = arcis_vec_new();
    if (out != NULL && out->len > 0) {
        ArcisString* h = arcis_string_from_cstr("gpu: ");
        ArcisString* line = arcis_string_concat(h, out);
        arcis_vec_push(v, (int64_t)(uintptr_t)line);
        arcis_string_drop(h);
    }
    arcis_string_drop(out);
    arcis_string_drop(cmd);
    return v;
}

double arcis_disk_free(ArcisString* path) {
    if (path == NULL) return 0.0;
    char tmp[4096]; snprintf(tmp, sizeof(tmp), "%.*s", (int)path->len, path->ptr);
    struct statvfs st;
    if (statvfs(tmp, &st) != 0) return 0.0;
    return (double)st.f_bavail * (double)st.f_frsize;
}

double arcis_net_online(void) {
    ArcisString* out = arcis_process_exec(arcis_string_from_cstr("ping -c1 -W3 1.1.1.1 2>/dev/null && echo 1 || echo 0"), NULL);
    if (out == NULL) return 0.0;
    double r = (out->len > 0 && out->ptr[0] == '1') ? 1.0 : 0.0;
    arcis_string_drop(out);
    return r;
}

ArcisString* arcis_net_public_ip(void) {
    return arcis_process_exec(arcis_string_from_cstr("curl -s --max-time 5 https://ifconfig.me 2>/dev/null"), NULL);
}

ArcisVec* arcis_net_interfaces(void) {
    return arcis_gpu_list(); // placeholder: reuse same parser structure
}

// ─────────────────────────────────────────────────────────────────────────────
// sys.args — std::env::args equivalent (Phase 5)
// ─────────────────────────────────────────────────────────────────────────────

// We store program args at startup. `main()` populates them before
// calling arcis_main().
static int    arcis_argc = 0;
static char** arcis_argv = NULL;

ArcisVec* arcis_sys_args(void) {
    ArcisVec* v = arcis_vec_new();
    for (int i = 0; i < arcis_argc; i++) {
        ArcisString* s = arcis_string_from_cstr(arcis_argv[i]);
        arcis_vec_push(v, (int64_t)(uintptr_t)s);
    }
    return v;
}

// ─────────────────────────────────────────────────────────────────────────────
// Platform entry stub.
//
// On every platform, the linker provides a `_start` (Linux ELF), `main`
// (Windows CRT), or equivalent that is the program's entry point. We
// define a C `main` here so the linker is happy regardless of which
// default entry the system uses; `main` simply calls the Cranelift-emitted
// `arcis_main` and returns. Phase 1 supports Linux x86_64/AArch64; macOS /
// Windows work too because the chain is the same.
// ─────────────────────────────────────────────────────────────────────────────

extern void arcis_main(void);

int main(int argc, char** argv) {
    arcis_argc = argc;
    arcis_argv = argv;
    arcis_main();
    return 0;
}
"#;
