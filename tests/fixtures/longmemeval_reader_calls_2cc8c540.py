def build_extra_body(args: argparse.Namespace) -> dict[str, Any] | None:
    extra_body: dict[str, Any] = {}
    if args.top_k is not None:
        extra_body["top_k"] = args.top_k
    if args.repetition_penalty is not None:
        extra_body["repetition_penalty"] = args.repetition_penalty
    if args.base_url and args.model == "Qwen/Qwen3.5-9B" and not args.reader_enable_thinking:
        extra_body["chat_template_kwargs"] = {"enable_thinking": False}
    return extra_body or None

def build_reader_request(
    args: argparse.Namespace,
    messages: list[dict[str, Any]],
) -> dict[str, Any]:
    req: dict[str, Any] = {
        "model": args.model,
        "messages": messages,
        "timeout": args.timeout_seconds,
    }
    if args.base_url:
        req["max_tokens"] = args.max_completion_tokens
    else:
        req["max_completion_tokens"] = args.max_completion_tokens
    if args.reasoning_effort is not None:
        req["reasoning_effort"] = args.reasoning_effort
    if args.temperature is not None:
        req["temperature"] = args.temperature
    if args.top_p is not None:
        req["top_p"] = args.top_p
    if args.presence_penalty is not None:
        req["presence_penalty"] = args.presence_penalty
    extra_body = build_extra_body(args)
    if extra_body is not None:
        req["extra_body"] = extra_body
    return req

def extract_usage_dict(response: Any) -> dict[str, int]:
    usage = getattr(response, "usage", None)
    return {
        "prompt_tokens": int(getattr(usage, "prompt_tokens", 0) or 0),
        "completion_tokens": int(getattr(usage, "completion_tokens", 0) or 0),
        "total_tokens": int(getattr(usage, "total_tokens", 0) or 0),
    }

async def call_reader_model_async(
    client: AsyncOpenAI,
    args: argparse.Namespace,
    messages: list[dict[str, Any]],
) -> tuple[str, dict[str, int]]:
    response = await client.chat.completions.create(**build_reader_request(args, messages))
    text = extract_text_from_response_message(response.choices[0].message)
    require(text != "", "Model returned empty text")
    return text, extract_usage_dict(response)
