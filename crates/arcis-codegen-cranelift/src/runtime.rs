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
    (void)argc;
    (void)argv;
    arcis_main();
    return 0;
}
"#;
