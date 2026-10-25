import { useEffect, useRef, useState } from "preact/hooks";

interface Subscription {
  name: string;
  url: string;
  behavior: "domain" | "ipcidr" | "classical";
  format: "yaml" | "text";
  policy: string;
  enabled: boolean;
}
interface RulesDocument {
  revision: string;
  rules: string;
  subscriptions: Subscription[];
  policies: string[];
  file_path?: string;
}
const errors: Record<number, string> = {
  400: "配置验证失败，请检查规则语法、订阅地址和策略。现有配置继续生效。",
  401: "登录已过期，请重新登录。",
  403: "当前账户没有修改规则的权限。",
  409: "其他位置已修改配置。请先重新读取，再合并你的修改。",
  503: "规则管理服务暂不可用，请稍后重试。",
};
const snapshot = (value: RulesDocument) =>
  JSON.stringify([value.rules, value.subscriptions]);

export function RulesEditor({
  id,
  name,
  token,
  onClose,
  onUnauthorized,
}: {
  id: string;
  name: string;
  token: string;
  onClose: () => void;
  onUnauthorized: () => void;
}) {
  const dialog = useRef<HTMLDialogElement>(null);
  const [document, setDocument] = useState<RulesDocument | null>(null);
  const [saved, setSaved] = useState("");
  const [busy, setBusy] = useState(true);
  const [error, setError] = useState("");
  const [notice, setNotice] = useState("");
  const [type, setType] = useState("DOMAIN-SUFFIX");
  const [match, setMatch] = useState("");
  const [policy, setPolicy] = useState("DIRECT");
  const dirty = !!document && snapshot(document) !== saved;
  const endpoint = `/api/plugins/${encodeURIComponent(id)}/rules`;
  const request = async (method: string, suffix = "", body?: unknown) => {
    const response = await fetch(endpoint + suffix, {
      method,
      credentials: "same-origin",
      cache: "no-store",
      headers: {
        ...(token ? { Authorization: `Bearer ${token}` } : {}),
        ...(body !== undefined ? { "Content-Type": "application/json" } : {}),
      },
      ...(body !== undefined ? { body: JSON.stringify(body) } : {}),
      signal: AbortSignal.timeout(30000),
    });
    if (!response.ok) {
      if (response.status === 401) {
        onUnauthorized();
        onClose();
      }
      throw new Error(
        errors[response.status] ?? "规则操作失败，请检查服务状态后重试。",
      );
    }
    return response;
  };
  const load = async () => {
    const response = await request("GET");
    const value = (await response.json()) as RulesDocument;
    setDocument(value);
    setSaved(snapshot(value));
    setPolicy(
      value.policies.includes("DIRECT")
        ? "DIRECT"
        : (value.policies[0] ?? "DIRECT"),
    );
  };
  useEffect(() => {
    dialog.current?.showModal();
    let active = true;
    void load()
      .catch((error) => {
        if (active)
          setError(
            error instanceof Error && error.name === "Error"
              ? error.message
              : "无法读取规则配置，请检查权限和服务状态后重试。",
          );
      })
      .finally(() => {
        if (active) setBusy(false);
      });
    return () => {
      active = false;
      dialog.current?.close();
    };
  }, []);
  useEffect(() => {
    const prevent = (event: BeforeUnloadEvent) => {
      if (dirty) {
        event.preventDefault();
        event.returnValue = "";
      }
    };
    window.addEventListener("beforeunload", prevent);
    return () => window.removeEventListener("beforeunload", prevent);
  }, [dirty]);
  const close = () => {
    if (!busy && (!dirty || window.confirm("放弃尚未保存的规则修改？")))
      onClose();
  };
  const perform = async (reload: boolean) => {
    if (busy || !document) return;
    if (
      reload &&
      dirty &&
      !window.confirm("重新读取将放弃尚未保存的修改，继续吗？")
    )
      return;
    setBusy(true);
    setError("");
    setNotice("");
    try {
      if (reload) await request("POST", "/reload", {});
      else
        await request("PUT", "", {
          revision: document.revision,
          rules: document.rules,
          subscriptions: document.subscriptions,
        });
      await load();
      setNotice(
        reload ? "已重新读取并应用手写配置。" : "规则已验证、应用并保存。",
      );
    } catch (error) {
      setError(
        error instanceof Error && error.name === "Error"
          ? error.message
          : "请求超时或网络中断，请重新读取确认当前配置。",
      );
    } finally {
      setBusy(false);
    }
  };
  const updateSubscription = (index: number, patch: Partial<Subscription>) => {
    if (document)
      setDocument({
        ...document,
        subscriptions: document.subscriptions.map((item, i) =>
          i === index ? { ...item, ...patch } : item,
        ),
      });
    setNotice("");
  };
  return (
    <dialog
      ref={dialog}
      class="rules-editor"
      aria-labelledby="rules-title"
      onCancel={(event) => {
        event.preventDefault();
        close();
      }}
    >
      <div class="rules-heading">
        <div>
          <div class="eyebrow">MIHOMO RULES</div>
          <h2 id="rules-title">{name} · 规则管理</h2>
        </div>
        <button type="button" class="refresh" disabled={busy} onClick={close}>
          关闭
        </button>
      </div>
      {error && (
        <div class="alert" role="alert">
          {error}
        </div>
      )}
      {notice && (
        <p role="status" class="mint">
          {notice}
        </p>
      )}
      {!document ? (
        <p>{busy ? "正在读取配置…" : "未能读取配置。"}</p>
      ) : (
        <form
          onSubmit={(event) => {
            event.preventDefault();
            void perform(false);
          }}
        >
          <fieldset disabled={busy}>
            <legend>手写规则</legend>
            <p class="rules-help">
              按从上到下的顺序匹配。也可 SSH 编辑
              {document.file_path ? (
                <code> {document.file_path} </code>
              ) : (
                "手写规则文件"
              )}
              后点“重新读取”。
            </p>
            <div class="rule-builder">
              <label>
                规则类型
                <select
                  aria-label="规则类型"
                  value={type}
                  onChange={(e) => setType(e.currentTarget.value)}
                >
                  {["DOMAIN", "DOMAIN-SUFFIX", "DOMAIN-KEYWORD", "IP-CIDR"].map(
                    (value) => (
                      <option key={value}>{value}</option>
                    ),
                  )}
                </select>
              </label>
              <label>
                匹配项
                <input
                  value={match}
                  placeholder={
                    type === "IP-CIDR" ? "192.0.2.0/24" : "example.com"
                  }
                  onInput={(e) => setMatch(e.currentTarget.value)}
                />
              </label>
              <label>
                目标策略
                <select
                  aria-label="目标策略"
                  value={policy}
                  onChange={(e) => setPolicy(e.currentTarget.value)}
                >
                  {document.policies.map((value) => (
                    <option key={value}>{value}</option>
                  ))}
                </select>
              </label>
              <button
                type="button"
                class="refresh"
                onClick={() => {
                  if (!match.trim() || /[,\r\n]/.test(match)) {
                    setError("请填写有效匹配项，不能包含逗号或换行。");
                    return;
                  }
                  const rule = `${type},${match.trim()},${policy}`;
                  setDocument({
                    ...document,
                    rules:
                      document.rules.replace(/\s*$/, "") +
                      (document.rules.trim() ? "\n" : "") +
                      rule +
                      "\n",
                  });
                  setMatch("");
                  setError("");
                  setNotice("");
                }}
              >
                添加规则
              </button>
            </div>
            <label class="rules-source">
              原始规则
              <textarea
                spellcheck={false}
                rows={10}
                value={document.rules}
                onInput={(e) => {
                  setDocument({ ...document, rules: e.currentTarget.value });
                  setNotice("");
                }}
              />
            </label>
          </fieldset>
          <fieldset disabled={busy}>
            <legend>规则订阅</legend>
            <p class="rules-help">
              订阅按列表顺序排列；禁用会保留配置。订阅 URL
              仅发送至路由器，不保存在浏览器存储中。
            </p>
            {document.subscriptions.map((subscription, index) => (
              <div
                class="rule-subscription"
                role="group"
                aria-label={`规则订阅 ${index + 1}`}
                key={index}
              >
                <label>
                  名称
                  <input
                    required
                    value={subscription.name}
                    onInput={(e) =>
                      updateSubscription(index, { name: e.currentTarget.value })
                    }
                  />
                </label>
                <label class="subscription-url">
                  订阅 URL
                  <input
                    required
                    type="url"
                    autoComplete="off"
                    value={subscription.url}
                    onInput={(e) =>
                      updateSubscription(index, { url: e.currentTarget.value })
                    }
                  />
                </label>
                <label>
                  匹配格式
                  <select
                    aria-label="匹配格式"
                    value={subscription.behavior}
                    onChange={(e) =>
                      updateSubscription(index, {
                        behavior: e.currentTarget
                          .value as Subscription["behavior"],
                      })
                    }
                  >
                    {["domain", "ipcidr", "classical"].map((value) => (
                      <option key={value}>{value}</option>
                    ))}
                  </select>
                </label>
                <label>
                  文件格式
                  <select
                    aria-label="文件格式"
                    value={subscription.format}
                    onChange={(e) =>
                      updateSubscription(index, {
                        format: e.currentTarget.value as Subscription["format"],
                      })
                    }
                  >
                    {["yaml", "text"].map((value) => (
                      <option key={value}>{value}</option>
                    ))}
                  </select>
                </label>
                <label>
                  订阅策略
                  <select
                    aria-label="订阅策略"
                    value={subscription.policy}
                    onChange={(e) =>
                      updateSubscription(index, {
                        policy: e.currentTarget.value,
                      })
                    }
                  >
                    {[
                      ...new Set([...document.policies, subscription.policy]),
                    ].map((value) => (
                      <option key={value}>{value}</option>
                    ))}
                  </select>
                </label>
                <label class="subscription-enabled">
                  <input
                    type="checkbox"
                    checked={subscription.enabled}
                    onChange={(e) =>
                      updateSubscription(index, {
                        enabled: e.currentTarget.checked,
                      })
                    }
                  />
                  启用
                </label>
                <button
                  type="button"
                  class="refresh"
                  onClick={() => {
                    setDocument({
                      ...document,
                      subscriptions: document.subscriptions.filter(
                        (_, i) => i !== index,
                      ),
                    });
                    setNotice("");
                  }}
                >
                  删除订阅
                </button>
              </div>
            ))}
            <button
              type="button"
              class="refresh"
              onClick={() => {
                let number = document.subscriptions.length + 1;
                while (
                  document.subscriptions.some(
                    (item) => item.name === `custom-${number}`,
                  )
                )
                  number++;
                setDocument({
                  ...document,
                  subscriptions: [
                    ...document.subscriptions,
                    {
                      name: `custom-${number}`,
                      url: "",
                      behavior: "domain",
                      format: "yaml",
                      policy: "REJECT",
                      enabled: true,
                    },
                  ],
                });
                setNotice("");
              }}
            >
              添加订阅
            </button>
          </fieldset>
          <div class="rules-actions">
            <span>{dirty ? "有未保存的修改" : "与已保存配置一致"}</span>
            <button
              type="button"
              class="refresh"
              disabled={busy}
              onClick={() => void perform(true)}
            >
              重新读取
            </button>
            <button type="submit" class="primary" disabled={busy || !dirty}>
              {busy ? "处理中…" : "验证并应用"}
            </button>
          </div>
        </form>
      )}
    </dialog>
  );
}
