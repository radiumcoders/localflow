// The whole cleanup engine as four C functions over llama.cpp: load a GGUF,
// complete a prompt greedily (optionally held to a GBNF grammar), free. Built as its own shared library with
// llama.cpp and ggml linked in and hidden, so its ggml can't clash with the
// (older) one whisper.cpp brings into localflow's binary.

#include <cmath>
#include <cstdio>
#include <cstdlib>
#include <cstring>
#include <mutex>
#include <string>
#include <vector>

#include "llama.h"

#if defined(_WIN32)
#define LF_API extern "C" __declspec(dllexport)
#else
#define LF_API extern "C" __attribute__((visibility("default")))
#endif

struct lf_model {
    llama_model * model;
    llama_context * ctx;
    llama_sampler * sampler;
};

static void quiet(ggml_log_level level, const char * text, void *) {
    if (level == GGML_LOG_LEVEL_ERROR || getenv("LOCALFLOW_LLM_LOG")) {
        fputs(text, stderr);
    }
}

static char * copy(const std::string & s) {
    char * out = (char *) malloc(s.size() + 1);
    memcpy(out, s.c_str(), s.size() + 1);
    return out;
}

// gpu: offload every layer (Vulkan/Metal, when there is a device); else CPU.
// n_ctx bounds prompt + answer. On failure, returns null and *error says why.
LF_API lf_model * lf_load(const char * path, int gpu, int n_ctx, char ** error) {
    static std::once_flag once;
    std::call_once(once, [] {
        llama_log_set(quiet, nullptr);
        llama_backend_init();
    });
    llama_model_params mp = llama_model_default_params();
    mp.n_gpu_layers = gpu ? -1 : 0;
    llama_model * model = llama_model_load_from_file(path, mp);
    if (!model) {
        *error = copy(std::string("couldn't load ") + path);
        return nullptr;
    }
    llama_context_params cp = llama_context_default_params();
    cp.n_ctx = n_ctx;
    cp.n_batch = n_ctx;  // the prompt goes in one batch
    cp.n_ubatch = 512;
    cp.no_perf = true;
    llama_context * ctx = llama_init_from_model(model, cp);
    if (!ctx) {
        llama_model_free(model);
        *error = copy("couldn't make a context for the model");
        return nullptr;
    }
    llama_sampler * sampler = llama_sampler_chain_init(llama_sampler_chain_default_params());
    llama_sampler_chain_add(sampler, llama_sampler_init_greedy());
    return new lf_model{model, ctx, sampler};
}

// The greedy choice the grammar allows. The model's own first choice usually
// is allowed, and checking one token is cheap; only when it isn't is the
// grammar run over the whole vocabulary (what llama.cpp's own sampler does).
static llama_token pick(llama_sampler * grammar, llama_context * ctx, const llama_vocab * vocab,
                        std::vector<llama_token_data> & all) {
    const float * logits = llama_get_logits_ith(ctx, -1);
    const int n = (int) all.size();
    llama_token best = 0;
    for (int t = 1; t < n; t++) {
        if (logits[t] > logits[best]) best = t;
    }
    llama_token_data one = {best, logits[best], 0.0f};
    llama_token_data_array single = {&one, 1, -1, false};
    llama_sampler_apply(grammar, &single);
    if (single.data[0].logit == -INFINITY) {
        for (int t = 0; t < n; t++) all[t] = {t, logits[t], 0.0f};
        llama_token_data_array cur = {all.data(), all.size(), -1, false};
        llama_sampler_apply(grammar, &cur);
        best = -1;
        for (size_t i = 0; i < cur.size; i++) {
            if (cur.data[i].logit != -INFINITY && (best < 0 || cur.data[i].logit > logits[best])) best = cur.data[i].id;
        }
        if (best < 0) return llama_vocab_eos(vocab);  // a dead end: stop there
    }
    // The end of the text needs no accepting (and llama.cpp aborts on one the grammar can't end at).
    if (!llama_vocab_is_eog(vocab, best)) llama_sampler_accept(grammar, best);
    return best;
}

// The greedy continuation of `prompt` (special tokens like <|im_start|> are
// parsed), up to the end-of-turn token or max_tokens. With a `grammar` (GBNF,
// rule "root"; null or empty for none), only text it allows can come out.
// *cut_off is set when it stopped at max_tokens. Null on failure, with *error set.
LF_API char * lf_complete(lf_model * m, const char * prompt, const char * grammar, int max_tokens, int * cut_off,
                          char ** error) {
    const llama_vocab * vocab = llama_model_get_vocab(m->model);
    llama_memory_clear(llama_get_memory(m->ctx), true);
    llama_sampler * sampler = m->sampler;
    llama_sampler * grammar_sampler = nullptr;
    if (grammar && *grammar) {
        grammar_sampler = llama_sampler_init_grammar(vocab, grammar, "root");
        if (!grammar_sampler) {
            *error = copy("the grammar doesn't parse");
            return nullptr;
        }
    }
    llama_sampler_reset(sampler);
    // The grammar made for this call is freed with it, on every return.
    struct Owned {
        llama_sampler * s;
        ~Owned() { if (s) llama_sampler_free(s); }
    } owned{grammar_sampler};
    const int n_vocab = llama_vocab_n_tokens(vocab);
    std::vector<llama_token_data> all(grammar_sampler ? n_vocab : 0);
    int len = (int) strlen(prompt);
    int n = -llama_tokenize(vocab, prompt, len, nullptr, 0, false, true);
    std::vector<llama_token> tokens(n);
    if (llama_tokenize(vocab, prompt, len, tokens.data(), n, false, true) < 0) {
        *error = copy("couldn't tokenize the prompt");
        return nullptr;
    }
    if (n + max_tokens > (int) llama_n_ctx(m->ctx)) {
        max_tokens = (int) llama_n_ctx(m->ctx) - n;
        if (max_tokens <= 0) {
            *error = copy("dictation too long for the model's context");
            return nullptr;
        }
    }
    std::string out;
    llama_batch batch = llama_batch_get_one(tokens.data(), n);
    llama_token next;
    *cut_off = 1;
    for (int i = 0; i < max_tokens; i++) {
        if (llama_decode(m->ctx, batch)) {
            *error = copy("the model failed to run");
            return nullptr;
        }
        next = grammar_sampler ? pick(grammar_sampler, m->ctx, vocab, all) : llama_sampler_sample(sampler, m->ctx, -1);
        if (llama_vocab_is_eog(vocab, next)) {
            *cut_off = 0;
            break;
        }
        char piece[256];
        int k = llama_token_to_piece(vocab, next, piece, sizeof(piece), 0, false);
        if (k > 0) {
            out.append(piece, k);
        }
        batch = llama_batch_get_one(&next, 1);
    }
    return copy(out);
}

LF_API void lf_free_string(char * s) {
    free(s);
}

LF_API void lf_free(lf_model * m) {
    llama_sampler_free(m->sampler);
    llama_free(m->ctx);
    llama_model_free(m->model);
    delete m;
}
