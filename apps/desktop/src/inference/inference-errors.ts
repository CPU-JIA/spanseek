type Translate = (zh: string, en: string) => string;
export function inferenceErrorMessage(error: unknown, t: Translate): string {
  const detail = (error as { details?: { details?: { kind?: string; reason?: string } } })?.details
    ?.details;
  const reason =
    typeof error === 'string' ? error : detail?.kind === 'inference_failure' ? detail.reason : null;
  const descriptions: Record<string, string> = {
    knowledge_task_input_invalid: t(
      '请填写任务目标；任务要求不能超过 8,000 字。',
      'Enter a task goal. Instructions must be no longer than 8,000 characters.',
    ),
    knowledge_task_materials_required: t(
      '请至少选择一个资料或笔记片段。',
      'Select at least one source or note excerpt.',
    ),
    knowledge_task_material_limit: t(
      '材料最多 32 个片段、合计 256 KiB，请减少材料后重试。',
      'Use at most 32 excerpts and 256 KiB of material. Reduce the selection and retry.',
    ),
    knowledge_task_period_invalid: t(
      '请检查任务的起止日期。',
      'Check the task start and end dates.',
    ),
    knowledge_task_retry_invalid: t(
      '原任务仍在运行，等待结束或停止后再试。',
      'The original task is still running. Wait or stop it before retrying.',
    ),
    task_template_missing: t(
      '所选模板已删除，请重新选择或移除模板关联，当前要求仍保留。',
      'The selected template was deleted. Choose another or remove the template association. Your instructions are retained.',
    ),
    task_template_duplicate_name: t(
      '已有同名模板，请使用不同名称。',
      'A template with this name already exists. Choose another name.',
    ),
    task_template_name_invalid: t(
      '模板名称需为 1–100 个字符。',
      'The template name must contain 1–100 characters.',
    ),
    task_template_content_invalid: t(
      '请填写任务要求（最多 8,000 字），说明最多 2,000 字。',
      'Enter instructions (up to 8,000 characters) and a description of up to 2,000 characters.',
    ),
    task_template_version_invalid: t(
      '模板版本已失效，请重新读取后再保存。',
      'The template version is invalid. Reload it before saving.',
    ),
    chat_evidence_invalid: t(
      '部分材料已变化或不在当前范围，请重新选择这些片段。',
      'Some materials have changed or are outside the current scope. Select those excerpts again.',
    ),
    answer_output_budget_invalid: t(
      '输出预算须为 1–32,768 的整数且小于模型上下文容量。请在问答的生成设置中调整，或检查模型上下文设置。',
      'Output budget must be an integer from 1 to 32,768 and below model context capacity. Adjust generation settings in Answers or check the model context setting.',
    ),
    remote_consent_required: t(
      '启用外部服务前，需要先确认知悉：启用后，参与检索与生成的相关文本会发送到你配置的服务。请在“模型与连接”页勾选知情同意后再保存。',
      'Before enabling an external service, acknowledge that related text will be sent to the service you configure. Tick the acknowledgement on the Models & connections page, then save again.',
    ),
    semantic_disabled: t(
      '本知识库已关闭语义检索，本次只使用本地关键词检索。可在设置中重新启用。',
      'Semantic search is disabled for this workspace, so only local keyword search ran. Re-enable it in settings.',
    ),
    embedding_unconfigured: t(
      '尚未配置知识库向量检索，语义索引不可用。请在模型设置中选择向量服务商与模型。',
      'Knowledge embeddings are not configured, so the semantic index is unavailable. Choose an embedding provider and model in model settings.',
    ),
    index_unavailable: t(
      '当前范围尚无可用向量索引，请检查索引进度或重建索引。',
      'No vector index is ready in this scope. Check progress or rebuild the index.',
    ),
    embedding_remote_required: t(
      '请在模型设置中配置向量服务并重建知识库向量索引。',
      'Configure embeddings in model settings and rebuild the vector index.',
    ),
    embedding_invalid_endpoint: t(
      '请填写有效的向量 API 地址：远程地址必须使用 HTTPS，本机地址可用 http://127.0.0.1、http://localhost 或 http://[::1]。',
      'Enter a valid embedding API URL: remote addresses must use HTTPS, while a local server may use http://127.0.0.1, http://localhost or http://[::1].',
    ),
    embedding_dimension_mismatch: t(
      '模型返回的向量维度与所选输出维度不一致，未保存。请改用该模型支持的维度或选择“默认”。',
      'The model returned a different vector size than the requested output size, so nothing was saved. Choose a supported size or “Default”.',
    ),
    embedding_unavailable: t(
      '向量服务暂时无法连接，请检查网络与 API 地址。',
      'The embedding service could not be reached. Check the network and API URL.',
    ),
    embedding_api_failed: t(
      '向量服务返回错误，本次索引或检索未完成。',
      'The embedding service returned an error. Indexing or search did not complete.',
    ),
    embedding_invalid_response: t(
      '向量服务返回的内容无效，本次结果未使用。',
      'The embedding response was invalid and was not used.',
    ),
    remote_authentication_failed: t(
      '向量服务认证失败，请检查令牌权限。',
      'Embedding authentication failed. Check the token permissions.',
    ),
    remote_rate_limited: t(
      '向量服务请求过于频繁，请稍后重试。',
      'The embedding service is rate limited. Try again later.',
    ),

    interrupted_unknown_outcome: t(
      '上次生成因应用退出而中断，模型请求的最终结果未知；不会自动重新调用模型。需要继续时请重新发起问答。',
      'The previous answer was interrupted when the app exited. The model request’s final outcome is unknown; the model will not be called again automatically. Start a new question to continue.',
    ),
    worker_interrupted: t(
      '上次运行因应用或后台进程退出而中断，已生成内容仍保留；不会自动重新调用模型。',
      'The previous run was interrupted when the app or background process exited. Generated text is retained; the model will not be called again automatically.',
    ),
    chat_restored_copy: t(
      '这是从备份恢复的未完成请求。已保留原内容，不会自动重新调用模型；需要继续时请发送新消息。',
      'This unfinished request came from a backup. Its content is retained and the model will not run again automatically. Send a new message to continue.',
    ),
    chat_context_limit: t(
      '输入、历史与输出预算超出上下文容量。请新建对话，或调整模型上下文与输出预算。',
      'Input, history and output exceed context capacity. Start a new conversation or adjust context and output limits.',
    ),
    chat_total_timeout: t(
      '已达到本次总时限，已生成内容仍保留。',
      'The total time limit was reached. Generated text is retained.',
    ),
    chat_model_budget_exhausted: t(
      '已达到模型请求次数上限，已生成内容仍保留。',
      'The model request limit was reached. Generated text is retained.',
    ),
    chat_tool_budget_exhausted: t(
      '已达到工具调用上限，已生成内容仍保留。',
      'The tool call limit was reached. Generated text is retained.',
    ),
    chat_output_limit: t(
      '已达到本次内容容量上限，已生成内容仍保留。',
      'The response size limit was reached. Generated text is retained.',
    ),
    agent_total_timeout: t(
      '知识助手已达到本次总时限，未发布未完成的答案。',
      'The assistant reached its total time limit. No incomplete answer was published.',
    ),
    agent_tool_budget_exhausted: t(
      '知识助手已用完本次工具调用预算，请缩小问题范围后重新发起。',
      'The assistant reached its tool-call limit. Narrow the question and start a new request.',
    ),
    agent_evidence_limit: t(
      '找到的证据超过本次容量上限，请缩小范围。',
      'The evidence exceeds this run’s capacity. Narrow the scope.',
    ),
    agent_evidence_invalid: t(
      '证据内容发生变化或未通过校验，本次助手已停止。',
      'Evidence changed or failed validation. The assistant stopped.',
    ),
    agent_evidence_handle_invalid: t(
      '模型请求读取无效的证据，已停止工具调用。',
      'The model requested an invalid evidence reference. Tool execution stopped.',
    ),
    agent_tool_invalid: t(
      '模型返回的工具参数无效，未执行该批调用。',
      'The model returned invalid tool arguments. This batch was not executed.',
    ),
    agent_tool_unknown: t(
      '模型请求了未授权的工具，未执行该批调用。',
      'The model requested an unauthorized tool. This batch was not executed.',
    ),
    agent_options_invalid: t(
      '助手运行上限无效，请重新发起请求。',
      'The assistant limits are invalid. Start a new request.',
    ),
    agent_request_invalid: t(
      '助手请求无法处理，请检查问题和模型配置。',
      'The assistant request could not be processed. Check the question and model configuration.',
    ),
    model_output_truncated: t(
      '模型未完整结束响应，本次结果未发布。',
      'The model response did not finish completely. No result was published.',
    ),
    model_total_timeout: t(
      '模型请求达到总时限，请稍后重新发起。',
      'The model request reached its total time limit. Start a new request later.',
    ),
    model_first_token_timeout: t(
      '模型未及时开始响应，请检查服务状态。',
      'The model did not start responding in time. Check the service status.',
    ),
    model_idle_timeout: t(
      '模型响应中断超时，本次结果未发布。',
      'The model response stalled. No result was published.',
    ),
    model_unconfigured: t(
      '请先在模型设置中配置可用模型。',
      'Configure an available model in Model settings first.',
    ),
    model_unavailable: t(
      '模型服务或网络不可用，请检查连接与服务状态。',
      'The model service or network is unavailable. Check the connection and service status.',
    ),
    model_key_missing: t(
      '此配置尚未保存 API 密钥，请先在模型设置中保存。',
      'No API key is saved for this configuration. Save one in model settings.',
    ),
    model_invalid_key: t(
      'API 密钥格式无效，请重新填写。',
      'The API key format is invalid. Enter it again.',
    ),
    model_credential_unavailable: t(
      '无法访问系统凭据存储，请检查系统状态后重试。',
      'System credential storage is unavailable. Check the system and retry.',
    ),
    model_credential_unsupported: t(
      '此系统尚不支持安全存储 API 密钥。',
      'Secure API key storage is not supported on this system.',
    ),
    model_authentication_failed: t(
      '服务拒绝了身份验证，请检查密钥及其访问权限。',
      'The service rejected authentication. Check the API key and its permissions.',
    ),
    model_rate_limited: t(
      '服务请求达到限额，请稍后重试或检查账户额度。',
      'The service request limit was reached. Retry later or check your account quota.',
    ),
    model_api_error: t(
      'API 服务返回错误，请检查服务状态后重试。',
      'The API service returned an error. Check its status and retry.',
    ),
    model_request_rejected: t(
      '服务商拒绝了这次请求，重试同样的请求不会改变结果：请检查渠道里的模型 ID、协议与请求参数（模型可能已停用或写错）。',
      'The provider rejected this request, so retrying it will not help. Check the model ID, protocol and parameters in this channel; the model may be retired or misspelled.',
    ),
    model_invalid_response: t(
      '服务返回的内容不符合所选协议，请检查接口协议与地址。',
      'The service response does not match the selected protocol. Check the protocol and endpoint.',
    ),
    model_response_too_large: t(
      '服务响应超过处理上限，请缩小请求范围后重试。',
      'The service response exceeded the processing limit. Narrow the request and retry.',
    ),
    model_incomplete: t(
      '服务响应未完整结束，本次结果未被接受，请重试。',
      'The service response did not finish completely. The result was not accepted; retry.',
    ),
    model_unsupported_output: t(
      '服务返回了当前不支持的输出类型，本次结果未被接受。',
      'The service returned an unsupported output type. The result was not accepted.',
    ),
    model_invalid_request: t(
      '请求参数无效，请检查模型配置与上下文容量。',
      'The request parameters are invalid. Check the model configuration and context capacity.',
    ),
    model_invalid_endpoint: t(
      '服务地址无效。远程 API 须使用 HTTPS，并填写准确的 API 根路径。',
      'The service address is invalid. Remote APIs require HTTPS and the exact API base path.',
    ),
    model_profile_unsupported: t(
      '此模型配置不受支持，请检查协议、模型名称与参数。',
      'This model configuration is unsupported. Check the protocol, model name, and parameters.',
    ),
    model_template_unsupported: t(
      '该模型的对话模板不受支持，不能保证上下文完整性。',
      'This chat template is unsupported; context integrity cannot be guaranteed.',
    ),
    model_context_limit: t(
      '问题、证据和生成预算超过上下文容量。请缩小范围或使用受支持的更大上下文配置。',
      'The question, evidence, and generation budget exceed the context capacity. Narrow the scope or use a supported larger context.',
    ),
    model_not_found: t(
      '本机服务中没有此模型，请先完成下载。',
      'This model is absent from the local service. Complete its download first.',
    ),
    embedding_model_unavailable: t(
      '固定 embedding 模型文件不可用，关键词检索仍可使用。',
      'Pinned embedding model files are unavailable. Keyword search remains available.',
    ),
    embedding_model_hash_mismatch: t(
      'embedding 文件校验失败，请修复固定版本模型文件。',
      'Embedding file verification failed. Repair the pinned model files.',
    ),
    embedding_profile_mismatch: t(
      '向量配置与索引身份不一致（升级后旧的后台索引组件仍在运行时最常见）。请重新保存向量配置并重建索引。',
      'The saved embedding configuration does not match the index identity (most often an older background index component is still running). Save the embedding configuration again and rebuild the index.',
    ),
    embedding_title_token_limit: t(
      '资料标题过长，已超过分块 token 上限。请缩短标题，或提高分块上限后重建索引。',
      'The source title is longer than the chunk token cap. Shorten the title or raise the chunk cap, then rebuild the index.',
    ),
    embedding_token_budget: t(
      '标题与正文合计超过分块 token 上限。请提高分块上限或缩短内容后重建索引。',
      'Title plus body exceed the chunk token cap. Raise the chunk cap or shorten the content, then rebuild the index.',
    ),
    embedding_chunk_size_limit: t(
      '单个分块超过允许的字节上限。请减小分块 token 上限后重建索引。',
      'A single chunk is larger than the allowed byte cap. Lower the chunk token cap and rebuild the index.',
    ),
    embedding_chunk_limit: t(
      '单篇资料的分块数超过上限（100000）。请缩短资料或降低分块上限后重建索引。',
      'One source produced more chunks than the limit (100,000). Shorten the source or lower the chunk cap, then rebuild the index.',
    ),
    embedding_batch_limit: t(
      '单次向量请求的输入条数超过上限，本次索引未完成。请重试；若反复出现，请降低分块上限。',
      'One embedding request carried more inputs than allowed, so this index pass did not complete. Retry, and lower the chunk cap if it repeats.',
    ),
    embedding_input_limit: t(
      '送入向量服务的文本超过长度上限，本次索引未完成。请降低分块上限后重建索引。',
      'Text sent to the embedding service exceeded its length limit, so this index pass did not complete. Lower the chunk cap and rebuild.',
    ),
    embedding_invalid_input: t(
      '索引输入无效：资料内容或结构已经变化。请重新导入该资料后重建索引。',
      'The index input is invalid because the source content or structure changed. Re-import the source and rebuild the index.',
    ),
    embedding_invalid_plan: t(
      '索引计划无效，本次索引未完成。请重建该资料的索引。',
      'The index plan is invalid, so this index pass did not complete. Rebuild the index for that source.',
    ),
    embedding_invalid_vector: t(
      '向量服务返回的向量无效（维度或数值异常），本次结果未使用。',
      'The embedding service returned an invalid vector (wrong size or non-finite value). This result was discarded.',
    ),
    embedding_invalid_vector_count: t(
      '向量服务返回的向量条数与请求不一致，本次结果未使用。请重试。',
      'The embedding service returned a different number of vectors than requested. This result was discarded; retry.',
    ),
    embedding_inference_failed: t(
      '向量计算失败，本次索引或检索未完成。请重试。',
      'Embedding computation failed, so this indexing or retrieval pass did not complete. Retry.',
    ),
    embedding_model_invalid: t(
      '本机固定 embedding 模型文件无效。关键词检索仍可使用；请修复模型文件或改用远程向量服务。',
      'The pinned local embedding model files are invalid. Keyword search still works; repair the model files or use a remote embedding service.',
    ),
    embedding_model_load_failed: t(
      '本机固定 embedding 模型加载失败。关键词检索仍可使用；请修复模型文件或改用远程向量服务。',
      'The pinned local embedding model failed to load. Keyword search still works; repair the model files or use a remote embedding service.',
    ),
    embedding_tokenizer_invalid: t(
      '本机分词器文件无效。关键词检索仍可使用；请修复分词器文件后重建索引。',
      'The local tokenizer files are invalid. Keyword search still works; repair the tokenizer files and rebuild the index.',
    ),
    embedding_tokenization_failed: t(
      '文本分词失败，本次索引或检索未完成。请重试。',
      'Tokenization failed, so this indexing or retrieval pass did not complete. Retry.',
    ),
    projection_incomplete: t(
      '向量索引写入未完成，本次索引不可用。请重建索引。',
      'The vector index write did not complete, so this index is unavailable. Rebuild the index.',
    ),
    vector_commit_failed: t(
      '向量索引写入失败，本次索引不可用。请重试或重建索引。',
      'Writing the vector index failed, so this index is unavailable. Retry or rebuild the index.',
    ),
    vector_open_failed: t(
      '无法打开本机向量索引。请重试，若仍失败请重建索引。',
      'The local vector index could not be opened. Retry, and rebuild the index if it keeps failing.',
    ),
    vector_query_failed: t(
      '向量检索失败，本次只返回了关键词结果。请重试。',
      'Vector retrieval failed, so this pass returned keyword results only. Retry.',
    ),
    inference_runtime_unavailable: t(
      '本机推理组件不可用（缺失或无法启动）。请重新安装或修复应用；关键词检索仍可使用。',
      'The local inference component is unavailable (missing or unable to start). Reinstall or repair the app; keyword search still works.',
    ),
    inference_invalid_request: t(
      '后台推理请求无效，本次操作未完成。请重试；若持续出现，请更新到最新版本。',
      'The background inference request was invalid, so this operation did not complete. Retry, and update if it keeps happening.',
    ),
    inference_protocol_error: t(
      '后台推理进程通信失败，本次操作未完成。请重试。',
      'Communication with the background inference process failed, so this operation did not complete. Retry.',
    ),
    inference_invalid_frame: t(
      '后台推理进程返回的数据帧无效，本次操作未完成。请重试。',
      'The background inference process returned an invalid frame, so this operation did not complete. Retry.',
    ),
    inference_event_limit: t(
      '后台推理进程返回的事件过大，本次操作未完成。请缩小范围后重试。',
      'The background inference process returned an oversized event, so this operation did not complete. Narrow the scope and retry.',
    ),
    inference_result_invalid: t(
      '后台推理结果无效，本次结果未使用。请重试。',
      'The background inference result was invalid and was discarded. Retry.',
    ),
    inference_result_limit: t(
      '后台推理结果超过上限，本次结果未使用。请缩小范围后重试。',
      'The background inference result exceeded the limit and was discarded. Narrow the scope and retry.',
    ),
    inference_result_unavailable: t(
      '后台推理结果不可用，本次操作未完成。请重试。',
      'The background inference result is unavailable, so this operation did not complete. Retry.',
    ),
    inference_output_unavailable: t(
      '后台推理的输出文件不可用，本次操作未完成。请重试。',
      'The background inference output file is unavailable, so this operation did not complete. Retry.',
    ),
    model_canceled: t(
      '本次模型请求已取消，未使用任何结果。',
      'This model request was canceled and no result was used.',
    ),
    ollama_unavailable: t(
      '本机模型服务不可用。请确认服务已启动，或改用远程 AI 渠道。',
      'The local model service is unavailable. Start the service or switch to a remote AI channel.',
    ),
    ollama_model_unavailable: t(
      '本机模型服务中没有此模型。请先完成下载。',
      'The local model service does not have this model. Download it first.',
    ),
    ollama_timeout: t(
      '本机模型服务响应超时。请重试，或改用更小的模型。',
      'The local model service timed out. Retry, or use a smaller model.',
    ),
  };
  const normalizedReason = reason === 'invalid_model_endpoint' ? 'model_invalid_endpoint' : reason;
  return normalizedReason && Object.hasOwn(descriptions, normalizedReason)
    ? descriptions[normalizedReason]
    : detail?.kind === 'inference_failure'
      ? t(
          '模型请求未完成，请检查配置后重试。',
          'The model request did not complete. Check the configuration and retry.',
        )
      : error instanceof Error
        ? error.message
        : t('操作未完成，请重试。', 'The operation did not complete. Retry.');
}
