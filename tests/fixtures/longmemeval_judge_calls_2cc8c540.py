def _call_chat_completion(
    *,
    client: Any,
    model: str,
    messages: list[dict[str, str]],
    max_completion_tokens: int,
    reasoning_effort: str | None,
    temperature: float | None,
    top_p: float | None,
    timeout_seconds: float,
) -> str:
    request: dict[str, Any] = {
        "model": model,
        "messages": messages,
        "max_completion_tokens": max_completion_tokens,
        "timeout": timeout_seconds,
    }
    if reasoning_effort is not None:
        request["reasoning_effort"] = reasoning_effort
    if temperature is not None:
        request["temperature"] = temperature
    if top_p is not None:
        request["top_p"] = top_p

    response = client.chat.completions.create(**request)
    message_content = response.choices[0].message.content
    if isinstance(message_content, str):
        return message_content.strip()
    if isinstance(message_content, list):
        text_parts = []
        for item in message_content:
            if isinstance(item, dict):
                text = item.get("text")
                if isinstance(text, str):
                    text_parts.append(text)
        joined = "\n".join(text_parts).strip()
        if joined:
            return joined
    raise ValueError("Evaluator model returned empty response content.")
