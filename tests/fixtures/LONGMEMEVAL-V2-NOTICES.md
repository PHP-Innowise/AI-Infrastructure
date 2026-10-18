# LongMemEval-V2 fixture provenance

The four Python fixtures below contain source from
[xiaowu0162/LongMemEval-V2, commit 2cc8c540bdb87fe6761629b585e727e1c4704520](https://github.com/xiaowu0162/LongMemEval-V2/tree/2cc8c540bdb87fe6761629b585e727e1c4704520).
They are licensed under **Apache License 2.0**. The complete upstream license
accompanies them in [LongMemEval-V2-LICENSE](LongMemEval-V2-LICENSE).
Include both companion files whenever redistributing these fixtures.

| Local fixture | Upstream source | Extraction |
| --- | --- | --- |
| `longmemeval_memory_api_2cc8c540.py` | [memory_modules/memory.py](https://github.com/xiaowu0162/LongMemEval-V2/blob/2cc8c540bdb87fe6761629b585e727e1c4704520/memory_modules/memory.py) | Complete byte-identical file. |
| `longmemeval_reader_calls_2cc8c540.py` | [evaluation/harness.py, lines 851–906](https://github.com/xiaowu0162/LongMemEval-V2/blob/2cc8c540bdb87fe6761629b585e727e1c4704520/evaluation/harness.py#L851) | Four request/call functions extracted; surrounding imports and harness omitted. |
| `longmemeval_reader_helpers_2cc8c540.py` | [evaluation/harness.py, lines 93–95 and 827–848](https://github.com/xiaowu0162/LongMemEval-V2/blob/2cc8c540bdb87fe6761629b585e727e1c4704520/evaluation/harness.py#L827) | `require` and the response extractor; surrounding harness omitted. |
| `longmemeval_judge_calls_2cc8c540.py` | [evaluation/qa_eval_metrics.py, lines 506–544](https://github.com/xiaowu0162/LongMemEval-V2/blob/2cc8c540bdb87fe6761629b585e727e1c4704520/evaluation/qa_eval_metrics.py#L506) | `_call_chat_completion` extracted; surrounding evaluator omitted. |

The extracted function texts retain the pinned upstream bytes, separated by
blank lines. Their extraction is the local adaptation; the functions have not
been edited. These executable excerpts contain no benchmark answers or gold.
The pinned upstream source files carry no copyright header, and its complete
64-entry tree contains no NOTICE file. No owner or year is inferred from the
generic copyright placeholder in the license appendix.

SHA-256 pins of the unmodified fixture images:

```text
607ca21320067df2abc3583b80debeb819781e90955776bdea4313a8b93a0586  longmemeval_memory_api_2cc8c540.py
06bb4d73591af5ea8cbaa11ad1cd94a51e49a25f900da8ff4de9433b3227f009  longmemeval_reader_calls_2cc8c540.py
2d2ebbcc6f7efed44f98f8194ddcb94cbc26df08a65d115a838a2cf9567e914d  longmemeval_reader_helpers_2cc8c540.py
42c17caa741deea9d7cc01d35aea2655fe9b2135cf97bf17cca9fc888c4b7786  longmemeval_judge_calls_2cc8c540.py
```

The accompanying license is the exact 11,357-byte upstream image, SHA-256
`c71d239df91726fc519c6eb72d318ec65820627232b2f796219e87dcf35d0ab4`.
This notice applies to these fixtures; it does not change the repository's
license or license the upstream dataset under a different profile.
