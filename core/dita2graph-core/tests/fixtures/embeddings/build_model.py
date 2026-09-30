import onnx
from onnx import helper, TensorProto, numpy_helper
import numpy as np
import json

VOCAB = {
    "[UNK]": 0,
    "install": 1, "installing": 2, "product": 3, "download": 4, "run": 5,
    "configure": 6, "configuration": 7, "settings": 8, "overview": 9,
    "weather": 10, "forecast": 11, "rain": 12, "cloud": 13, "sunny": 14, "temperature": 15,
}
DIM = 8
rng = np.random.default_rng(42)

def cluster_vec(base, jitter_scale=0.05):
    v = np.array(base, dtype=np.float32)
    v = v + rng.normal(0, jitter_scale, size=DIM).astype(np.float32)
    return v

table = np.zeros((len(VOCAB), DIM), dtype=np.float32)
install_base = [1, 1, 0, 0, 0, 0, 0, 0]
config_base = [0, 0, 1, 1, 0, 0, 0, 0]
weather_base = [0, 0, 0, 0, 1, 1, 0, 0]
for tok in ["install", "installing", "product", "download", "run"]:
    table[VOCAB[tok]] = cluster_vec(install_base)
for tok in ["configure", "configuration", "settings", "overview"]:
    table[VOCAB[tok]] = cluster_vec(config_base)
for tok in ["weather", "forecast", "rain", "cloud", "sunny", "temperature"]:
    table[VOCAB[tok]] = cluster_vec(weather_base)
# [UNK] stays zero

embedding_init = numpy_helper.from_array(table, name="embedding_table")

input_ids = helper.make_tensor_value_info("input_ids", TensorProto.INT64, ["batch", "seq"])
attention_mask = helper.make_tensor_value_info("attention_mask", TensorProto.INT64, ["batch", "seq"])
output = helper.make_tensor_value_info("last_hidden_state", TensorProto.FLOAT, ["batch", "seq", DIM])

gather_node = helper.make_node(
    "Gather",
    inputs=["embedding_table", "input_ids"],
    outputs=["last_hidden_state"],
    axis=0,
    name="gather_embeddings",
)

graph = helper.make_graph(
    nodes=[gather_node],
    name="ToyEmbeddingModel",
    inputs=[input_ids, attention_mask],
    outputs=[output],
    initializer=[embedding_init],
)

model = helper.make_model(graph, producer_name="dita2graph-test-fixture", opset_imports=[helper.make_opsetid("", 18)])
model.ir_version = 9
onnx.checker.check_model(model)
onnx.save(model, "tiny-embedding-model.onnx")
print("saved onnx model")

tokenizer = {
    "version": "1.0",
    "truncation": None,
    "padding": None,
    "added_tokens": [],
    "normalizer": {"type": "Lowercase"},
    "pre_tokenizer": {"type": "Whitespace"},
    "post_processor": None,
    "decoder": None,
    "model": {
        "type": "WordLevel",
        "vocab": VOCAB,
        "unk_token": "[UNK]",
    },
}
with open("tokenizer.json", "w") as f:
    json.dump(tokenizer, f, indent=2)
print("saved tokenizer.json")
