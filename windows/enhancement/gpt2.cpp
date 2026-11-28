// GPT-2 compute graph derived from ggml-org/ggml bb877c0c51f38c52ee2d5e41d58e3d0c20f3fed4.
// Copyright (c) 2022-2026 The ggml authors. MIT; see third_party/licenses/ggml-LICENSE.txt.
// FreeType adds GGUF loading, a tree attention mask and a bounded C ABI.
#include <ggml.h>
#include <windows.h>
#include <ggml-alloc.h>
#include <ggml-backend.h>
#include <ggml-cpu.h>
#include <gguf.h>
#include <algorithm>
#include <cmath>
#include <cstdint>
#include <cstring>
#include <fstream>
#include <filesystem>
#include <map>
#include <memory>
#include <stdexcept>
#include <string>
#include <vector>
#define GPT2_MAX_NODES 4096
#define FT_API extern "C" __declspec(dllexport)
struct gpt2_hparams {
    int32_t n_vocab = 50257;
    int32_t n_ctx   = 1024;
    int32_t n_embd  = 768;
    int32_t n_head  = 12;
    int32_t n_layer = 12;
    int32_t ftype   = 1;
    float   eps     = 1e-5f;
};

struct gpt2_layer {
    // normalization
    struct ggml_tensor * ln_1_g;
    struct ggml_tensor * ln_1_b;

    struct ggml_tensor * ln_2_g;
    struct ggml_tensor * ln_2_b;

    // attention
    struct ggml_tensor * c_attn_attn_w;
    struct ggml_tensor * c_attn_attn_b;

    struct ggml_tensor * c_attn_proj_w;
    struct ggml_tensor * c_attn_proj_b;

    // mlp
    struct ggml_tensor * c_mlp_fc_w;
    struct ggml_tensor * c_mlp_fc_b;

    struct ggml_tensor * c_mlp_proj_w;
    struct ggml_tensor * c_mlp_proj_b;
};

struct gpt2_model {
    gpt2_hparams hparams;

    // normalization
    struct ggml_tensor * ln_f_g;
    struct ggml_tensor * ln_f_b;

    struct ggml_tensor * wte;     //    token embedding
    struct ggml_tensor * wpe;     // position embedding
    struct ggml_tensor * lm_head; // language model head

    std::vector<gpt2_layer> layers;

    // key + value memory
    struct ggml_tensor * memory_k;
    struct ggml_tensor * memory_v;

    //
    struct ggml_context * ctx_w;
    struct ggml_context * ctx_kv;

    ggml_backend_t backend = NULL;

    ggml_backend_buffer_t buffer_w;
    ggml_backend_buffer_t buffer_kv;

    std::map<std::string, struct ggml_tensor *> tensors;
};

// build the computation graph
struct ggml_cgraph * gpt2_graph(
        const gpt2_model & model,
        const int n_past,
        const int n_tokens) {
    const int N = n_tokens;

    const auto & hparams = model.hparams;

    const int n_embd  = hparams.n_embd;
    const int n_layer = hparams.n_layer;
    const int n_ctx   = hparams.n_ctx;
    const int n_head  = hparams.n_head;

    // since we are using ggml-alloc, this buffer only needs enough space to hold the ggml_tensor and ggml_cgraph structs, but not the tensor data
    size_t buf_size = ggml_tensor_overhead()*GPT2_MAX_NODES + ggml_graph_overhead_custom(GPT2_MAX_NODES, false);
    thread_local std::vector<uint8_t> buf; buf.resize(buf_size);

    struct ggml_init_params params = {
        /*.mem_size   =*/ buf_size,
        /*.mem_buffer =*/ buf.data(),
        /*.no_alloc   =*/ true, // the tensors will be allocated later by ggml_gallocr_alloc_graph()
    };

    struct ggml_context * ctx = ggml_init(params);

    struct ggml_cgraph  * gf = ggml_new_graph_custom(ctx, GPT2_MAX_NODES, false);

    struct ggml_tensor * embd = ggml_new_tensor_1d(ctx, GGML_TYPE_I32, N);
    // at this point, the tensor data is not allocated yet and cannot be set
    // we will find the tensor after the graph is allocated by its name, and set the data then
    ggml_set_name(embd, "embd");
    // setting a tensor as an input will ensure that it is allocated at the beginning of the graph
    // this is important to ensure that the input tensors are not overwritten before they are used
    ggml_set_input(embd);

    struct ggml_tensor * position = ggml_new_tensor_1d(ctx, GGML_TYPE_I32, N);
    ggml_set_name(position, "position");
    ggml_set_input(position);

    // wte + wpe
    struct ggml_tensor * inpL =
        ggml_add(ctx,
                ggml_get_rows(ctx, model.wte, embd),
                ggml_get_rows(ctx, model.wpe, position));

    // KQ_mask = mask_past()
    // [n_past + N, N, 1]
    const int n_kv = n_past + N;

    struct ggml_tensor * KQ_mask = ggml_new_tensor_3d(ctx, GGML_TYPE_F32, n_kv, N, 1);
    ggml_set_name(KQ_mask, "KQ_mask");
    ggml_set_input(KQ_mask);

    for (int il = 0; il < n_layer; ++il) {
        struct ggml_tensor * cur;

        // norm
        {
            // [ 768, N]
            cur = ggml_norm(ctx, inpL, hparams.eps);

            // cur = ln_1_g*cur + ln_1_b
            // [ 768, N]
            cur = ggml_add(ctx,
                    ggml_mul(ctx,
                        cur,
                        model.layers[il].ln_1_g),
                    model.layers[il].ln_1_b);
        }

        // attn
        // [2304, 768] - model.layers[il].c_attn_attn_w
        // [2304,   1] - model.layers[il].c_attn_attn_b
        // [ 768,   N] - cur (in)
        // [2304,   N] - cur (out)
        //
        // cur = attn_w*cur + attn_b
        // [2304, N]
        {
            cur = ggml_mul_mat(ctx,
                    model.layers[il].c_attn_attn_w,
                    cur);

            cur = ggml_add(ctx,
                    cur,
                    model.layers[il].c_attn_attn_b);
        }

        // self-attention
        {
            struct ggml_tensor * Qcur = ggml_view_2d(ctx, cur, n_embd, N, cur->nb[1], 0*sizeof(float)*n_embd);
            struct ggml_tensor * Kcur = ggml_view_2d(ctx, cur, n_embd, N, cur->nb[1], 1*sizeof(float)*n_embd);
            struct ggml_tensor * Vcur = ggml_view_2d(ctx, cur, n_embd, N, cur->nb[1], 2*sizeof(float)*n_embd);

            // store key and value to memory
            if (N >= 1) {
                struct ggml_tensor * k = ggml_view_1d(ctx, model.memory_k, N*n_embd, (ggml_element_size(model.memory_k)*n_embd)*(il*n_ctx + n_past));
                struct ggml_tensor * v = ggml_view_1d(ctx, model.memory_v, N*n_embd, (ggml_element_size(model.memory_v)*n_embd)*(il*n_ctx + n_past));

                ggml_build_forward_expand(gf, ggml_cpy(ctx, Kcur, k));
                ggml_build_forward_expand(gf, ggml_cpy(ctx, Vcur, v));
            }

            // Q = Qcur.contiguous().view(n_embd/n_head, n_head, N).permute(0, 2, 1, 3)
            // [64, N, 12]
            struct ggml_tensor * Q =
                ggml_permute(ctx,
                        ggml_cont_3d(ctx, Qcur, n_embd/n_head, n_head, N),
                        0, 2, 1, 3);

            // K = Kmem.view(n_embd/n_head, n_head, n_past + N).permute(0, 2, 1, 3)
            // [64, n_past + N, 12]
            struct ggml_tensor * K =
                ggml_permute(ctx,
                        ggml_reshape_3d(ctx,
                            ggml_view_1d(ctx, model.memory_k, (n_past + N)*n_embd, il*n_ctx*ggml_element_size(model.memory_k)*n_embd),
                            n_embd/n_head, n_head, n_past + N),
                        0, 2, 1, 3);

            // GG: flash attention
            //struct ggml_tensor * V =
            //    ggml_cpy(ctx0,
            //            ggml_permute(ctx0,
            //                ggml_reshape_3d(ctx0,
            //                    ggml_view_1d(ctx0, model.memory_v, (n_past + N)*n_embd, il*n_ctx*ggml_element_size(model.memory_v)*n_embd),
            //                    n_embd/n_head, n_head, n_past + N),
            //                1, 2, 0, 3),
            //            ggml_new_tensor_3d(ctx0, GGML_TYPE_F32, n_past + N, n_embd/n_head, n_head));

            //struct ggml_tensor * KQV = ggml_flash_attn(ctx0, Q, K, V, true);

            // K * Q
            // [n_past + N, N, 12]
            struct ggml_tensor * KQ = ggml_mul_mat(ctx, K, Q);

            // KQ_scaled = KQ / sqrt(n_embd/n_head)
            // [n_past + N, N, 12]
            struct ggml_tensor * KQ_scaled =
                ggml_scale(ctx,
                        KQ,
                        1.0f/sqrtf(float(n_embd)/n_head));

            // KQ_masked = mask_past(KQ_scaled)
            // [n_past + N, N, 12]
            struct ggml_tensor * KQ_masked = ggml_add(ctx, KQ_scaled, KQ_mask);

            // KQ = soft_max(KQ_masked)
            // [n_past + N, N, 12]
            struct ggml_tensor * KQ_soft_max = ggml_soft_max(ctx, KQ_masked);

            // V_trans = Vmem.view(n_embd/n_head, n_head, n_past + N).permute(1, 2, 0, 3).contiguous()
            // [n_past + N, 64, 12]
            struct ggml_tensor * V_trans =
                ggml_cont_3d(ctx,
                        ggml_permute(ctx,
                            ggml_reshape_3d(ctx,
                                ggml_view_1d(ctx, model.memory_v, (n_past + N)*n_embd, il*n_ctx*ggml_element_size(model.memory_v)*n_embd),
                                n_embd/n_head, n_head, n_past + N),
                            1, 2, 0, 3),
                        n_past + N, n_embd/n_head, n_head);

            // KQV = transpose(V) * KQ_soft_max
            // [64, N, 12]
            struct ggml_tensor * KQV = ggml_mul_mat(ctx, V_trans, KQ_soft_max);

            // KQV_merged = KQV.permute(0, 2, 1, 3)
            // [64, 12, N]
            struct ggml_tensor * KQV_merged = ggml_permute(ctx, KQV, 0, 2, 1, 3);

            // cur = KQV_merged.contiguous().view(n_embd, N)
            // [768, N]
            cur = ggml_cont_2d(ctx, KQV_merged, n_embd, N);
        }

        // projection
        // [ 768, 768] - model.layers[il].c_attn_proj_w
        // [ 768,   1] - model.layers[il].c_attn_proj_b
        // [ 768,   N] - cur (in)
        // [ 768,   N] - cur (out)
        //
        // cur = proj_w*cur + proj_b
        // [768, N]
        {
            cur = ggml_mul_mat(ctx,
                    model.layers[il].c_attn_proj_w,
                    cur);

            cur = ggml_add(ctx,
                    cur,
                    model.layers[il].c_attn_proj_b);
        }

        // add the input
        cur = ggml_add(ctx, cur, inpL);

        struct ggml_tensor * inpFF = cur;

        // feed-forward network
        {
            // norm
            {
                cur = ggml_norm(ctx, inpFF, hparams.eps);

                // cur = ln_2_g*cur + ln_2_b
                // [ 768, N]
                cur = ggml_add(ctx,
                        ggml_mul(ctx,
                            cur,
                            model.layers[il].ln_2_g),
                        model.layers[il].ln_2_b);
            }

            // fully connected
            // [3072, 768] - model.layers[il].c_mlp_fc_w
            // [3072,   1] - model.layers[il].c_mlp_fc_b
            // [ 768,   N] - cur (in)
            // [3072,   N] - cur (out)
            //
            // cur = fc_w*cur + fc_b
            // [3072, N]
            cur = ggml_mul_mat(ctx,
                    model.layers[il].c_mlp_fc_w,
                    cur);

            cur = ggml_add(ctx,
                    cur,
                    model.layers[il].c_mlp_fc_b);

            // GELU activation
            // [3072, N]
            cur = ggml_gelu(ctx, cur);

            // projection
            // [ 768, 3072] - model.layers[il].c_mlp_proj_w
            // [ 768,    1] - model.layers[il].c_mlp_proj_b
            // [3072,    N] - cur (in)
            // [ 768,    N] - cur (out)
            //
            // cur = proj_w*cur + proj_b
            // [768, N]
            cur = ggml_mul_mat(ctx,
                    model.layers[il].c_mlp_proj_w,
                    cur);

            cur = ggml_add(ctx,
                    cur,
                    model.layers[il].c_mlp_proj_b);
        }

        // input for next layer
        inpL = ggml_add(ctx, cur, inpFF);
        if (il == 5) { ggml_set_name(inpL, "auxiliary_hidden"); ggml_set_output(inpL); ggml_build_forward_expand(gf, inpL); }
    }

    // norm
    {
        // [ 768, N]
        inpL = ggml_norm(ctx, inpL, hparams.eps);

        // inpL = ln_f_g*inpL + ln_f_b
        // [ 768, N]
        inpL = ggml_add(ctx,
                ggml_mul(ctx,
                    inpL,
                    model.ln_f_g),
                model.ln_f_b);
    }

    // inpL = WTE * inpL
    // [ 768, 50257] - model.lm_head
    // [ 768, N]     - inpL
    inpL = ggml_mul_mat(ctx, model.lm_head, inpL);
    ggml_set_name(inpL, "logits");
    // setting a tensor as the output will ensure that it is not overwritten by subsequent operations
    ggml_set_output(inpL);

    // logits -> probs
    //inpL = ggml_soft_max(ctx0, inpL);

    ggml_build_forward_expand(gf, inpL);

    ggml_free(ctx);

    return gf;
}

namespace {
constexpr int capacity = 2048;
struct Runtime {
    gpt2_model m{};
    gguf_context * file = nullptr;
    ggml_gallocr_t alloc = nullptr;
    ggml_threadpool_t pool = nullptr;
    std::vector<std::string> tokens;
    std::vector<int> parents, positions;
    std::vector<std::vector<float>> logits, hidden;
    std::vector<float> normalizers;
    ~Runtime() {
        if (alloc) ggml_gallocr_free(alloc);
        if (m.buffer_kv) ggml_backend_buffer_free(m.buffer_kv);
        if (m.buffer_w) ggml_backend_buffer_free(m.buffer_w);
        if (m.ctx_kv) ggml_free(m.ctx_kv);
        if (m.ctx_w) ggml_free(m.ctx_w);
        if (file) gguf_free(file);
        if (m.backend) ggml_backend_free(m.backend);
        if (pool) ggml_threadpool_free(pool);
    }
    ggml_tensor * weight(const std::string & name, int64_t a, int64_t b = 1) {
        auto * t = ggml_get_tensor(m.ctx_w, name.c_str());
        if (!t || t->ne[0] != a || t->ne[1] != b || t->ne[2] != 1 || t->ne[3] != 1)
            throw std::runtime_error("Invalid tensor: " + name);
        return t;
    }
    uint32_t integer(const char * name) {
        auto key = gguf_find_key(file, name);
        if (key < 0 || gguf_get_kv_type(file,key) != GGUF_TYPE_UINT32) throw std::runtime_error("Invalid model metadata");
        return gguf_get_val_u32(file,key);
    }
    void load(const wchar_t * path, int threads) {
        // Latency-sensitive inference is lower priority than input, but must not
        // inherit Windows' energy-saving clock throttling from persistence work.
        THREAD_POWER_THROTTLING_STATE power{THREAD_POWER_THROTTLING_CURRENT_VERSION,THREAD_POWER_THROTTLING_EXECUTION_SPEED,0};
        if(!SetThreadInformation(GetCurrentThread(),ThreadPowerThrottling,&power,sizeof(power)))
            throw std::runtime_error("Cannot configure model thread power policy");
        auto * input = _wfopen(path,L"rb");
        if (!input) throw std::runtime_error("Cannot open model");
        file = gguf_init_from_file_ptr(input,{true,&m.ctx_w});
        fclose(input);
        if (!file) throw std::runtime_error("Invalid GGUF");
        if (integer("gpt2.embedding_length")!=768 || integer("gpt2.block_count")!=12 || integer("gpt2.attention.head_count")!=12)
            throw std::runtime_error("Unsupported GPT-2 dimensions");
        auto key=gguf_find_key(file,"tokenizer.ggml.tokens");
        if (key<0 || gguf_get_kv_type(file,key)!=GGUF_TYPE_ARRAY || gguf_get_arr_type(file,key)!=GGUF_TYPE_STRING)
            throw std::runtime_error("Missing token vocabulary");
        auto count=gguf_get_arr_n(file,key);
        if (count!=21128 || integer("gpt2.context_length")!=1024) throw std::runtime_error("Unsupported vocabulary");
        for (size_t i=0;i<count;i++) tokens.emplace_back(gguf_get_arr_str(file,key,i));
        m.hparams.n_vocab=(int)count;m.hparams.n_ctx=capacity;
        m.backend=ggml_backend_cpu_init();
        if (!m.backend) throw std::runtime_error("CPU backend unavailable");
        ggml_backend_cpu_set_n_threads(m.backend,std::clamp(threads,1,8));
        auto pool_parameters=ggml_threadpool_params_default(std::clamp(threads,1,8));
        pool_parameters.prio=GGML_SCHED_PRIO_LOW;pool_parameters.poll=50;
        pool=ggml_threadpool_new(&pool_parameters);
        if(!pool)throw std::runtime_error("Inference thread pool unavailable");
        ggml_backend_cpu_set_threadpool(m.backend,pool);
        m.buffer_w=ggml_backend_alloc_ctx_tensors(m.ctx_w,m.backend);
        if (!m.buffer_w) throw std::runtime_error("Model allocation failed");
        std::ifstream stream(std::filesystem::path(path),std::ios::binary);
        auto file_size=std::filesystem::file_size(path);
        for (int64_t i=0;i<gguf_get_n_tensors(file);i++) {
            auto * t=ggml_get_tensor(m.ctx_w,gguf_get_tensor_name(file,i));
            auto offset=gguf_get_data_offset(file)+gguf_get_tensor_offset(file,i);
            auto bytes=ggml_nbytes(t);
            if (offset>file_size || bytes>file_size-offset || bytes>128*1024*1024) throw std::runtime_error("Tensor outside file");
            std::vector<char> buffer(bytes);stream.seekg(offset);stream.read(buffer.data(),bytes);
            if (!stream) throw std::runtime_error("Incomplete tensor");
            ggml_backend_tensor_set(t,buffer.data(),0,bytes);
        }
        m.wte=weight("token_embd.weight",768,count);m.lm_head=m.wte;
        m.wpe=weight("position_embd.weight",768,1024);
        m.ln_f_g=weight("output_norm.weight",768);m.ln_f_b=weight("output_norm.bias",768);
        for (int i=0;i<12;i++) {
            auto p="blk."+std::to_string(i)+".";gpt2_layer l{};
            l.ln_1_g=weight(p+"attn_norm.weight",768);l.ln_1_b=weight(p+"attn_norm.bias",768);
            l.ln_2_g=weight(p+"ffn_norm.weight",768);l.ln_2_b=weight(p+"ffn_norm.bias",768);
            l.c_attn_attn_w=weight(p+"attn_qkv.weight",768,2304);l.c_attn_attn_b=weight(p+"attn_qkv.bias",2304);
            l.c_attn_proj_w=weight(p+"attn_output.weight",768,768);l.c_attn_proj_b=weight(p+"attn_output.bias",768);
            l.c_mlp_fc_w=weight(p+"ffn_up.weight",768,3072);l.c_mlp_fc_b=weight(p+"ffn_up.bias",3072);
            l.c_mlp_proj_w=weight(p+"ffn_down.weight",3072,768);l.c_mlp_proj_b=weight(p+"ffn_down.bias",768);
            m.layers.push_back(l);
        }
        // GGUF auxiliary heads are standard two-layer MLPs over layer-six states.
        for (const auto & p : {std::string("ent"),std::string("seg")}) {
            weight(p+".fc1.w",2306,256);weight(p+".fc1.bias",256);
            weight(p+".fc2.w",256,p=="ent"?5:2);weight(p+".fc2.bias",p=="ent"?5:2);
        }
        m.ctx_kv=ggml_init({ggml_tensor_overhead()*4,nullptr,true});
        m.memory_k=ggml_new_tensor_1d(m.ctx_kv,GGML_TYPE_F32,12LL*capacity*768);
        m.memory_v=ggml_new_tensor_1d(m.ctx_kv,GGML_TYPE_F32,12LL*capacity*768);
        m.buffer_kv=ggml_backend_alloc_ctx_tensors(m.ctx_kv,m.backend);
        if (!m.buffer_kv) throw std::runtime_error("Cache allocation failed");
        alloc=ggml_gallocr_new(ggml_backend_get_default_buffer_type(m.backend));
    }
    void clear(int keep) {
        if (keep<0 || keep>(int)parents.size()) throw std::runtime_error("Invalid cache truncation");
        parents.resize(keep);positions.resize(keep);logits.resize(keep);hidden.resize(keep);normalizers.resize(keep);
    }
    void add(const int * token,const int * parent,int n) {
        ggml_threadpool_resume(pool);
        int base=(int)parents.size();
        if (n<1 || n>256 || base+n>capacity) throw std::runtime_error("Cache capacity exceeded");
        for(int i=0;i<n;i++) {
            if(token[i]<0 || token[i]>=m.hparams.n_vocab || parent[i]>=base+i || parent[i]<-1) throw std::runtime_error("Invalid model node");
            int pos=parent[i]<0?0:positions[parent[i]]+1;
            if(pos>=1024) throw std::runtime_error("Context too long");
            parents.push_back(parent[i]);positions.push_back(pos);
        }
        auto * graph=gpt2_graph(m,base,n);
        if(!ggml_gallocr_alloc_graph(alloc,graph)) throw std::runtime_error("Compute allocation failed");
        ggml_backend_tensor_set(ggml_graph_get_tensor(graph,"embd"),token,0,n*sizeof(int));
        ggml_backend_tensor_set(ggml_graph_get_tensor(graph,"position"),positions.data()+base,0,n*sizeof(int));
        std::vector<float> mask((base+n)*n,-INFINITY);
        for(int i=0;i<n;i++)for(int node=base+i;node>=0;node=parents[node])mask[i*(base+n)+node]=0;
        ggml_backend_tensor_set(ggml_graph_get_tensor(graph,"KQ_mask"),mask.data(),0,mask.size()*sizeof(float));
        if(ggml_backend_graph_compute(m.backend,graph)!=GGML_STATUS_SUCCESS)throw std::runtime_error("Model computation failed");
        auto * output=ggml_graph_get_tensor(graph,"logits");auto * aux=ggml_graph_get_tensor(graph,"auxiliary_hidden");
        for(int i=0;i<n;i++) {
            std::vector<float> row(m.hparams.n_vocab),h(768);
            ggml_backend_tensor_get(output,row.data(),i*row.size()*sizeof(float),row.size()*sizeof(float));
            ggml_backend_tensor_get(aux,h.data(),i*768*sizeof(float),768*sizeof(float));
            float peak=*std::max_element(row.begin(),row.end());double sum=0;
            for(float v:row)sum+=std::exp(v-peak);
            normalizers.push_back(peak+(float)std::log(sum));logits.push_back(std::move(row));hidden.push_back(std::move(h));
        }
    }
    void heads(const int * nodes,int n,int kind,float * result) {
        std::string p=kind?"seg":"ent";int classes=kind?2:5;
        auto * w1=(float*)weight(p+".fc1.w",2306,256)->data;auto * b1=(float*)weight(p+".fc1.bias",256)->data;
        auto * w2=(float*)weight(p+".fc2.w",256,classes)->data;auto * b2=(float*)weight(p+".fc2.bias",classes)->data;
        std::vector<std::vector<float>> norm(n,std::vector<float>(768));
        for(int i=0;i<n;i++) {
            if(nodes[i]<0||nodes[i]>=(int)hidden.size())throw std::runtime_error("Invalid head node");
            auto & h=hidden[nodes[i]];double mean=0,var=0;for(float x:h)mean+=x;mean/=768;
            for(float x:h)var+=(x-mean)*(x-mean);var=1/std::sqrt(var/768+1e-5);
            for(int k=0;k<768;k++)norm[i][k]=(float)((h[k]-mean)*var);
        }
        for(int i=0;i<n;i++) {
            float intermediate[256];
            for(int j=0;j<256;j++) {
                double x=b1[j];auto * weights=w1+j*2306;
                for(int d=0;d<3&&i+d<n;d++)for(int k=0;k<768;k++)x+=weights[d*768+k]*norm[i+d][k];
                if(i+1<n)x+=weights[2304];if(i+2<n)x+=weights[2305];
                intermediate[j]=(float)(.5*x*(1+std::erf(x/std::sqrt(2.0))));
            }
            float peak=-INFINITY;double sum=0;
            for(int c=0;c<classes;c++){float x=b2[c];for(int j=0;j<256;j++)x+=w2[c*256+j]*intermediate[j];result[i*classes+c]=x;peak=std::max(peak,x);}
            for(int c=0;c<classes;c++)sum+=(result[i*classes+c]=std::exp(result[i*classes+c]-peak));
            for(int c=0;c<classes;c++)result[i*classes+c]/=(float)sum;
        }
    }
};
thread_local std::string last_error;
template<class F> int guarded(F f){try{f();return 1;}catch(const std::exception & e){last_error=e.what();return 0;}}
}
FT_API const char * ft_model_error(){return last_error.c_str();}
FT_API void * ft_model_load(const wchar_t * path,int threads){auto p=std::make_unique<Runtime>();if(!guarded([&]{p->load(path,threads);}))return nullptr;return p.release();}
FT_API void ft_model_free(void * p){delete static_cast<Runtime*>(p);}
FT_API void ft_model_idle(void * p){ggml_threadpool_pause(static_cast<Runtime*>(p)->pool);}
FT_API const char * ft_model_token(void * p,int i){auto & t=static_cast<Runtime*>(p)->tokens;return i>=0&&i<(int)t.size()?t[i].c_str():nullptr;}
FT_API int ft_model_clear(void * p,int keep){return guarded([&]{static_cast<Runtime*>(p)->clear(keep);});}
FT_API int ft_model_add(void * p,const int * tokens,const int * parents,int n){return guarded([&]{static_cast<Runtime*>(p)->add(tokens,parents,n);});}
FT_API float ft_model_logp(void * p,int node,int token){auto * r=static_cast<Runtime*>(p);return node>=0&&node<(int)r->logits.size()&&token>=0&&token<r->m.hparams.n_vocab?r->logits[node][token]-r->normalizers[node]:-INFINITY;}
FT_API int ft_model_heads(void * p,const int * nodes,int n,int kind,float * output){return guarded([&]{static_cast<Runtime*>(p)->heads(nodes,n,kind,output);});}
