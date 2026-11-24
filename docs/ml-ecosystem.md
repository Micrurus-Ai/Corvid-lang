# Machine learning, deep learning and vision in Corvid

Readers: backend and AI development, and anyone judging whether a machine learning application is better built in Corvid than in Python.

Machine learning work fails in ways a compiler could have caught or a runtime could have recorded: a tensor shape mismatch hours into a training run, a model fed columns in the wrong order, results nobody can reproduce, a prediction nobody can trace to the model that made it, and a model that works in a notebook but is hard to ship. Corvid's advantages answer each of these. This page lists the libraries and frameworks Corvid reaches, how, and the stronger version Corvid gives each one. Like [Beyond LangChain, LangGraph and LangSmith](langchain-parity.md), every row's stronger version is the target, delivered by the task named.

## How Corvid reaches the ecosystem

Corvid's runtime is written in Rust, so it reaches the ecosystem two ways:

1. **Built in.** Libraries with a Rust or C core are embedded in the Corvid runtime. A model runs inside a single Corvid binary with no Python installed: fast to start, simple to deploy, and the same on a server and a laptop.
2. **Typed interoperability.** Everything else (PyTorch training, JAX, scikit-learn, spaCy and the rest of the Python ecosystem) is called through typed Python imports (B02, accepted by B07), with effects, approval, taint and replay applied to every call.

## The Corvid advantages applied to machine learning

| Advantage | What it means for machine learning |
| --- | --- |
| The compiler knows types | Tensor shapes, dataframe columns, image formats, embedding dimensions and a model's inputs and outputs are checked before anything runs. |
| A model file is a typed function | Importing an ONNX or GGUF model reads its signature, so calling it with the wrong shape is a compile error. |
| Effects and budgets | GPU time, memory, latency and cost are declared dimensions, proven or enforced like model-call cost today. |
| Provenance | Every prediction carries the model, version and weights hash that produced it, and every dataset carries where it came from. |
| Untrusted data is typed | Text from OCR, speech recognition or documents is `Tainted`, so instructions hidden in an image or audio clip cannot reach a dangerous action. |
| Personal data is typed | Training data and media containing personal data must pass a declared redaction step before training, logging or a model (B33). |
| Exact replay | Seeds, data snapshots and model versions are recorded, so a training run or an inference can be reproduced exactly. |
| Approval | Promoting a model or retraining on new data is a consequential action that requires approval and passing evaluations. |
| One program from interface to model | A browser camera, a vision model and the overlay on screen are one typed program, and people's corrections in the application become typed training data. |
| Declared placement | The same model runs on the device, in the browser or in the cloud according to a policy the source declares (B37). |

## Running models

| Library | Reached through | Corvid's stronger version | Task |
| --- | --- | --- | --- |
| ONNX Runtime | Built in | A model file imports as a typed function whose input and output tensor shapes come from the file, so a wrong shape is a compile error. Every prediction carries the model's identity, and latency and GPU budgets are declared. | B51 |
| llama.cpp and GGUF | Built in | Local language models in the same binary, with context length known to the compiler and output constrained to the declared type. | B51, B54 |
| whisper.cpp | Built in | Speech-to-text as a typed stream of segments to the interface; transcripts are untrusted by default. | B51 |
| Candle, Burn | Built in | Rust inference, and later training, with the checked tensor type. | B34, B51 |
| Hugging Face `tokenizers`, `safetensors` | Built in | Token counts known before a call, so prompt length and cost are checked against the model's limit. | B51 |
| Core ML, MLX, OpenVINO, TensorRT | Built in per platform | The same model call placed on the device's accelerator by declared policy. | B37, B51 |
| WebLLM and browser WebGPU | Built in for the web target | Inference in the user's browser when the declared placement allows it, with the cloud as the declared fallback. | B37 |
| vLLM, SGLang, TGI, Triton | HTTP or typed interop | Served models behind the same typed model call, with routing, budgets and replay. | B05, B17 |

## Deep learning and training

| Library | Reached through | Corvid's stronger version | Task |
| --- | --- | --- | --- |
| PyTorch | Typed interop, then native tensors | Tensors whose shapes, including symbolic dimensions such as `Batch`, are checked at compile time, so a mismatch fails in seconds instead of hours into a run. | B02, B34 |
| JAX, Flax, Optax | Typed interop | The same checked tensor type at the boundary, with traced and replayable calls. | B02, B34 |
| TensorFlow, Keras | Typed interop | Models exported to ONNX run natively through B51; training calls are typed and traced. | B02, B51 |
| Automatic differentiation | Native | `grad` over checked tensor functions, verified against numerical gradients. | B35 |
| Training runs | Native and interop | Training is declared durable work: checkpoints, pause and resume across restarts, a GPU-time budget, recorded seeds and data snapshots, so any run can be reproduced. | B59 |
| Hugging Face Transformers, Diffusers, sentence-transformers | Typed interop, ONNX export | Pretrained models called with typed inputs and outputs, exported to ONNX to run without Python in production. | B02, B51 |
| PEFT, TRL | Typed interop | Fine-tuning as durable, budgeted work whose dataset and result carry provenance and licenses. | B02, B56, B59 |

## Classical machine learning

| Library | Reached through | Corvid's stronger version | Task |
| --- | --- | --- | --- |
| scikit-learn, XGBoost, LightGBM, CatBoost | Typed interop, ONNX export | A trained model records the feature schema it expects, so passing columns that do not match is a compile error, and exported models run natively. | B02, B51, B52 |
| statsmodels, Prophet | Typed interop | Forecasts are typed values with their model's provenance. | B02 |
| SHAP | Typed interop | Explanations attached to the prediction they explain, kept with its provenance. | B02, B56 |
| Optuna | Typed interop | Hyperparameter search within a declared budget, with every trial recorded and replayable. | B02, B59 |

## Data

| Library | Reached through | Corvid's stronger version | Task |
| --- | --- | --- | --- |
| Polars, Apache Arrow | Built in | Dataframes whose column names and types are known to the compiler, read from Parquet or CSV schemas at compile time, so renaming a column lists every use. | B52 |
| DuckDB | Built in | Analytical SQL whose result columns are typed, as B09 does for application tables. | B09, B52 |
| NumPy, pandas | Typed interop | Arrays and frames cross the boundary as checked tensors and dataframes. | B02, B52 |
| Ray, Dask, Spark | Typed interop | Distributed jobs as durable work with budgets and provenance. | B02, B13 |

## Computer vision

| Library | Reached through | Corvid's stronger version | Task |
| --- | --- | --- | --- |
| OpenCV | Built in through its Rust bindings | Images are typed by format, channels and size, so feeding a grayscale image to a color model is a compile error. | B53 |
| Ultralytics YOLO, Segment Anything, MediaPipe | ONNX through B51, or typed interop | Detections and masks are typed values with the model's provenance, and a browser camera, the model and the on-screen overlay are one program. | B51, B53 |
| Tesseract, PaddleOCR | Built in or typed interop | Recognized text is untrusted by default, so instructions planted in an image are contained. | B53 |
| torchvision, Kornia, albumentations | Typed interop | Transform pipelines that keep the declared image type. | B02, B53 |
| Faces and people | Any of the above | Images containing people are personal data under B33: their storage, logging and use in training follow the declared rules. | B33, B53 |

## Speech and language

| Library | Reached through | Corvid's stronger version | Task |
| --- | --- | --- | --- |
| Whisper, faster-whisper | Built in (whisper.cpp) or interop | A typed stream of timed segments to the interface; transcripts untrusted by default. | B51 |
| pyannote | Typed interop | Speaker segments as typed values; voices are personal data. | B02, B33 |
| Piper, Coqui text-to-speech | Typed interop | Generated audio as a typed value with budget and provenance. | B02, B17 |
| spaCy, Stanza, NLTK | Typed interop | Entities and annotations as typed records with their source span. | B02 |

## Search and embeddings

| Library | Reached through | Corvid's stronger version | Task |
| --- | --- | --- | --- |
| Embedding models | Built in or provider | An embedding's type records the model and dimension that produced it, so searching an index with embeddings from a different model is a compile error. | B57 |
| Tantivy, HNSW indexes, LanceDB, FAISS | Built in | Keyword, vector and hybrid search inside the binary, with results carrying `Grounded` provenance. | B17, B57 |
| Qdrant, Weaviate, Milvus, Chroma, pgvector | Client | The same typed retrieval interface over each, with provenance kept. | B17 |

## Generation and prompt programs

| Library | Corvid's stronger version | Task |
| --- | --- | --- |
| Outlines, constrained decoding | A local model is constrained to generate only values of the declared type, so malformed output cannot happen rather than being repaired. | B54 |
| DSPy | Prompts are optimized against the program's own `eval` declarations within a declared budget, and every candidate run is recorded and replayable. | B55 |
| Instructor, Pydantic AI, Guardrails | Typed prompt output, refinements and repair are already language features. | Shipped |

## Experiments, registries and feedback

| Library | Corvid's stronger version | Task |
| --- | --- | --- |
| MLflow, Weights & Biases | Every run is already recorded, so experiments are compared from traces. A model registry stores typed model artifacts with their weights hash, license, training data lineage and evaluation scores. | B56 |
| DVC | Datasets are versioned by content hash, and every model and prediction names the dataset version behind it. | B56 |
| Model promotion | Promoting a model is a consequential action: it requires passing evaluations against the recorded baseline and an approval. | B49, B56 |
| BentoML, Kubeflow | A model is served as a typed action of the application, deployed with it as one binary. | B10, B43, B51 |
| Labelling and human feedback | People correct predictions in the application; corrections become typed training and evaluation data with provenance and consent, without interface code. | B58 |

## Checklist

- [ ] B51: native model inference with typed model imports (ONNX Runtime, GGUF, whisper.cpp), tokenizers and safetensors, in a single binary.
- [ ] B52: dataframes with compile-time schemas (Polars, Arrow, DuckDB) and model feature schemas.
- [ ] B53: typed images, OpenCV operations, detection and OCR with untrusted recognized text.
- [ ] B54: generation constrained to declared types for local models.
- [ ] B55: prompt optimization against `eval` declarations within a budget.
- [ ] B56: model registry and experiment tracking from traces, with weights hash, license, data lineage and eval-gated, approved promotion.
- [ ] B57: embedding types that record their model and dimension.
- [ ] B58: in-application corrections become typed training and evaluation data (with frontend proposal F-P8).
- [ ] B59: training as durable, budgeted, reproducible work.
- [ ] B02, B07: typed interop with PyTorch, JAX, TensorFlow, Transformers, scikit-learn, XGBoost, spaCy and the rest of the list.
- [ ] B34 to B37: checked tensors, `grad`, MLIR code generation and declared placement.
