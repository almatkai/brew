use crate::{cli::Executor, model, Adapter};
use serde_json::{json, Value};
use std::{
    collections::HashMap,
    sync::{
        atomic::{AtomicU64, Ordering},
        Arc,
    },
    time::Duration,
};
use tezbar_extension_sdk::{CoreResult, ExtensionContext};
#[derive(Clone)]
enum Callback {
    Refresh,
    Hide,
    Service(Vec<String>),
}
#[derive(Default)]
struct State {
    prefs: Value,
    callbacks: HashMap<String, Callback>,
}
#[derive(Default)]
pub(crate) struct Menu {
    generation: AtomicU64,
    state: tokio::sync::Mutex<State>,
    refresh: tokio::sync::Mutex<()>,
}
impl Menu {
    pub fn stop(&self) {
        self.generation.fetch_add(1, Ordering::SeqCst);
    }
}
async fn set(
    ctx: &ExtensionContext,
    executor: &Arc<dyn Executor>,
    menu: &Menu,
    generation: u64,
) -> CoreResult {
    let _lane = menu.refresh.lock().await;
    set_locked(ctx, executor, menu, generation).await
}
async fn set_locked(
    ctx: &ExtensionContext,
    executor: &Arc<dyn Executor>,
    menu: &Menu,
    generation: u64,
) -> CoreResult {
    if menu.generation.load(Ordering::SeqCst) != generation {
        return Ok(json!({"stale":true}));
    }
    let prefs = menu.state.lock().await.prefs.clone();
    let args = vec!["services".into(), "list".into(), "--json".into()];
    let rows = model::services(executor.run(ctx, &prefs, &args, false).await?.json(false)?)?;
    let mut callbacks = HashMap::new();
    let mut items = Vec::new();
    let mut item = |title: String, callback: Callback| {
        let id = uuid::Uuid::new_v4().to_string();
        callbacks.insert(id.clone(), callback);
        json!({"id":id,"title":title})
    };
    for verb in ["start", "stop", "restart"] {
        items.push(item(
            format!("{verb} all services"),
            Callback::Service(vec!["services".into(), verb.into(), "--all".into()]),
        ));
    }
    for row in rows.iter().take(28) {
        let mut children = Vec::new();
        for verb in ["start", "stop", "restart"] {
            children.push(item(
                verb.into(),
                Callback::Service(vec![
                    "services".into(),
                    verb.into(),
                    model::checked_name(&row["name"])?,
                ]),
            ));
        }
        items.push(json!({"id":uuid::Uuid::new_v4().to_string(),"kind":"submenu","title":format!("{} ({})",model::text(row,"name"),model::text(row,"description")),"items":children}));
    }
    if rows.len() > 28 {
        items.push(json!({"id":"overflow","kind":"section","title":format!("{} more services — use Manage Services",rows.len()-28)}));
    }
    items.push(item("Refresh".into(), Callback::Refresh));
    items.push(item("Hide Services Menu".into(), Callback::Hide));
    let running = rows
        .iter()
        .filter(|row| ["started", "scheduled"].contains(&row["description"].as_str().unwrap_or("")))
        .count();
    if menu.generation.load(Ordering::SeqCst) != generation {
        return Ok(json!({"stale":true}));
    }
    ctx.host("host.tray.set",json!({"resourceId":"brew-services","title":format!("Brew {running}"),"tooltip":"Homebrew Services","items":items})).await?;
    menu.state.lock().await.callbacks = callbacks;
    Ok(json!({"updated":true}))
}
async fn hide(ctx: &ExtensionContext, menu: &Menu, generation: u64) -> CoreResult {
    let _lane = menu.refresh.lock().await;
    if menu.generation.load(Ordering::SeqCst) != generation {
        return Ok(json!({"stale":true}));
    }
    menu.stop();
    menu.state.lock().await.callbacks.clear();
    ctx.host("host.background.release", json!({"key":"brew-services"}))
        .await?;
    ctx.host("host.tray.remove", json!({"resourceId":"brew-services"}))
        .await?;
    Ok(json!({"hidden":true}))
}
impl Adapter {
    pub(crate) async fn enable_menu(&self, ctx: &ExtensionContext, prefs: Value) -> CoreResult {
        let root = self
            .root
            .lock()
            .unwrap()
            .clone()
            .ok_or("Services menu requires initialized root context")?;
        let _lane = self.menu.refresh.lock().await;
        self.menu.stop();
        let generation = self.menu.generation.load(Ordering::SeqCst);
        self.menu.state.lock().await.prefs = prefs.clone();
        ctx.host(
            "host.background.retain",
            json!({"key":"brew-services","durable":true,"data":prefs}),
        )
        .await?;
        if let Err(error) = set_locked(&root, &self.executor, &self.menu, generation).await {
            self.menu.state.lock().await.callbacks.clear();
            let _ = root
                .host("host.tray.remove", json!({"resourceId":"brew-services"}))
                .await;
            let _ = ctx
                .host("host.background.release", json!({"key":"brew-services"}))
                .await;
            return Err(error);
        }
        let menu = self.menu.clone();
        let executor = self.executor.clone();
        let worker = root.clone();
        root.spawn(async move {
            loop {
                tokio::time::sleep(Duration::from_secs(60)).await;
                if menu.generation.load(Ordering::SeqCst)!=generation {break}
                if let Err(error)=set(&worker,&executor,&menu,generation).await {
                    let _lane = menu.refresh.lock().await;
                    if menu.generation.load(Ordering::SeqCst) != generation {break}
                    menu.state.lock().await.callbacks.clear();
                    let _=worker.host("host.tray.set",json!({"resourceId":"brew-services","title":"Brew !","tooltip":error,"items":[{"id":"failed","kind":"section","title":"Service refresh failed; rerun Services Menu"}]})).await;
                }
            }
        })?;
        Ok(json!({"enabled":true}))
    }
    pub(crate) async fn menu_event(&self, ctx: &ExtensionContext, payload: &Value) -> CoreResult {
        if payload["resourceId"] != "brew-services" {
            return Err("Unknown tray resource".into());
        }
        let (callback, prefs, generation) = {
            let _lane = self.menu.refresh.lock().await;
            let mut state = self.menu.state.lock().await;
            let callback = state
                .callbacks
                .remove(payload["actionId"].as_str().unwrap_or(""))
                .ok_or("Stale tray action")?;
            (
                callback,
                state.prefs.clone(),
                self.menu.generation.load(Ordering::SeqCst),
            )
        };
        match callback {
            Callback::Hide => {
                hide(ctx, &self.menu, generation).await?;
            }
            Callback::Refresh => {
                set(ctx, &self.executor, &self.menu, generation).await?;
            }
            Callback::Service(args) => {
                self.mutate(ctx, &prefs, args, false).await?;
                set(ctx, &self.executor, &self.menu, generation).await?;
            }
        }
        Ok(json!({"ok":true}))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::cli::Output;
    use std::future::Future;
    use tezbar_extension_sdk::{BoxFuture, ExtensionPaths, TestContext};
    struct Delayed {
        entered: tokio::sync::Notify,
        resume: tokio::sync::Notify,
    }
    impl Executor for Delayed {
        fn run<'a>(
            &'a self,
            _: &'a ExtensionContext,
            _: &'a Value,
            _: &'a [String],
            _: bool,
        ) -> BoxFuture<'a, Result<Output, String>> {
            Box::pin(async move {
                self.entered.notify_one();
                self.resume.notified().await;
                Ok(Output {
                    code: 0,
                    stdout: "[]".into(),
                    stderr: String::new(),
                })
            })
        }
    }
    #[tokio::test]
    async fn hide_waits_for_publication_and_fences_late_refreshes() {
        let dir = tempfile::tempdir().unwrap();
        let test = TestContext::new(
            ExtensionPaths {
                package_dir: dir.path().into(),
                data_dir: dir.path().into(),
                temp_dir: dir.path().into(),
            },
            &["tray.set", "tray.remove", "background.release"],
        );
        let menu = Menu::default();
        let delay = Arc::new(Delayed {
            entered: Default::default(),
            resume: Default::default(),
        });
        let executor: Arc<dyn Executor> = delay.clone();
        test.run(async {
            let refreshing = set(&test.context, &executor, &menu, 0);
            let hiding = async {
                delay.entered.notified().await;
                let future = hide(&test.context, &menu, 0);
                tokio::pin!(future);
                // Poll Hide during the delayed CLI read: it must wait for the
                // publication lane, then remove the resource as the last effect.
                std::future::poll_fn(|cx| {
                    assert!(future.as_mut().poll(cx).is_pending());
                    std::task::Poll::Ready(())
                })
                .await;
                delay.resume.notify_one();
                future.await
            };
            let (updated, hidden) = tokio::join!(refreshing, hiding);
            assert_eq!(updated?["updated"], true);
            assert_eq!(hidden?["hidden"], true);
            assert_eq!(
                set(&test.context, &executor, &menu, 0).await?["stale"],
                true
            );
            assert!(menu.state.lock().await.callbacks.is_empty());
            Ok::<Value, String>(json!({}))
        })
        .await
        .unwrap();
        let methods: Vec<_> = test
            .calls()
            .iter()
            .map(|call| call["method"].as_str().unwrap().to_owned())
            .collect();
        assert_eq!(
            methods,
            [
                "host.tray.set",
                "host.background.release",
                "host.tray.remove"
            ]
        );
    }
}
