use rynna_core::{
    CacheOptimizer, CompletionRequest, Message, PrefixCacheOptimizer, ToolDefinition,
};
use serde_json::json;

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
