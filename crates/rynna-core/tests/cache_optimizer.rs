use rynna_core::{
    CacheOptimization, CacheOptimizer, CompletionRequest, Message, PrefixCacheOptimizer,
    ToolDefinition,
};
use serde_json::json;

#[test]
fn server_cache_key_preserves_sha256_hex_encoding() {
    let cache = CacheOptimization {
        use_server_cache: true,
        scope_key: "abc".to_owned(),
    };

    assert_eq!(
        cache.server_cache_key(),
        "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"
    );
}

#[test]
fn prefix_cache_scope_preserves_sha256_hex_encoding() {
    let cache = PrefixCacheOptimizer.optimize(&CompletionRequest {
        messages: vec![],
        tools: vec![],
    });

    assert_eq!(
        cache.scope_key,
        "771298dc0a13ef497ad01cde7f7bb54281f569f7af73824c222a709ced050544"
    );
}

#[test]
fn prefix_cache_optimizer_keeps_a_stable_scope_as_conversation_grows() {
    let optimizer = PrefixCacheOptimizer;
    let tools = vec![ToolDefinition::new(
        "read_file",
        "Read a file",
        json!({"type": "object"}),
    )];
    let first = CompletionRequest {
        messages: vec![Message::system("Stable policy"), Message::user("First")],
        tools: tools.clone(),
    };
    let later = CompletionRequest {
        messages: vec![
            Message::system("Stable policy"),
            Message::user("First"),
            Message::assistant("Answer"),
            Message::user("Second"),
        ],
        tools,
    };

    let first = optimizer.optimize(&first);
    let later = optimizer.optimize(&later);

    assert!(first.use_server_cache);
    assert_eq!(first.scope_key, later.scope_key);
}

#[test]
fn prefix_cache_optimizer_separates_different_stable_prefixes() {
    let optimizer = PrefixCacheOptimizer;
    let first = CompletionRequest {
        messages: vec![Message::system("Policy A"), Message::user("Hello")],
        tools: vec![],
    };
    let second = CompletionRequest {
        messages: vec![Message::system("Policy B"), Message::user("Hello")],
        tools: vec![],
    };

    assert_ne!(
        optimizer.optimize(&first).scope_key,
        optimizer.optimize(&second).scope_key
    );
}

#[test]
fn prefix_cache_optimizer_separates_different_conversation_anchors() {
    let optimizer = PrefixCacheOptimizer;
    let first = CompletionRequest {
        messages: vec![Message::system("Policy"), Message::user("First task")],
        tools: vec![],
    };
    let second = CompletionRequest {
        messages: vec![Message::system("Policy"), Message::user("Second task")],
        tools: vec![],
    };

    assert_ne!(
        optimizer.optimize(&first).scope_key,
        optimizer.optimize(&second).scope_key
    );
}

#[test]
fn prefix_cache_optimizer_domain_separates_system_messages() {
    let optimizer = PrefixCacheOptimizer;
    let first = CompletionRequest {
        messages: vec![Message::system("a\0b"), Message::system("c")],
        tools: vec![],
    };
    let second = CompletionRequest {
        messages: vec![Message::system("a"), Message::system("b\0c")],
        tools: vec![],
    };

    assert_ne!(
        optimizer.optimize(&first).scope_key,
        optimizer.optimize(&second).scope_key
    );
}

#[test]
fn prefix_cache_optimizer_domain_separates_system_content_from_the_anchor() {
    let optimizer = PrefixCacheOptimizer;
    let anchor = Message::user("Task");
    let serialized_anchor = serde_json::to_string(&anchor).unwrap();
    let system_only = CompletionRequest {
        messages: vec![Message::system(serialized_anchor)],
        tools: vec![],
    };
    let anchored = CompletionRequest {
        messages: vec![anchor],
        tools: vec![],
    };

    assert_ne!(
        optimizer.optimize(&system_only).scope_key,
        optimizer.optimize(&anchored).scope_key
    );
}
