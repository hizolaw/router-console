import { render } from "preact";
import { useEffect, useRef, useState } from "preact/hooks";
import type { ComponentChildren } from "preact";
import "./style.css";

type Health = "healthy" | "degraded" | "failed" | "unknown";
type Kind = "system" | "http" | "dns" | "link" | "subscription";
interface Metric {
  label: string;
  value: string;
  unit: string;
}
interface Plugin {
  id: string;
  name: string;
  kind: Kind;
  visible: boolean;
  url: string | null;
  health: Health;
  summary: string;
  metrics: Metric[];
  checked_at: number | null;
  stale_after_secs: number;
}
interface Status {
  title: string;
  version: string;
  generated_at: number;
  plugins: Plugin[];
}
const labels: Record<Health, string> = {
  healthy: "正常",
  degraded: "降级",
  failed: "异常",
  unknown: "未知",
};
const kinds: Record<Kind, string> = {
  system: "SYSTEM",
  http: "HTTP CHECK",
  dns: "DNS CHECK",
  link: "QUICK LINK",
  subscription: "SUBSCRIPTION",
};
const icons: Record<Kind, string> = {
  system: "▦",
  http: "⇄",
  dns: "◎",
  link: "↗",
  subscription: "⟳",
};

function Icon({ children }: { children: ComponentChildren }) {
  return (
    <span class="icon" aria-hidden="true">
      {children}
    </span>
  );
}
function Badge({ health }: { health: Health }) {
  return (
    <span class={`badge ${health}`}>
      <i />
      {labels[health]}
    </span>
  );
}
function time(value: number | null) {
  return value
    ? new Date(value * 1000).toLocaleTimeString("zh-CN", { hour12: false })
    : "等待检查";
}

function Card({ plugin }: { plugin: Plugin }) {
  return (
    <article class={`card ${plugin.health}`}>
      <div class="card-top">
        <span class="card-icon">
          <Icon>{icons[plugin.kind]}</Icon>
        </span>
        <Badge health={plugin.health} />
      </div>
      <div class="eyebrow">{kinds[plugin.kind]}</div>
      <h2>{plugin.name}</h2>
      <p class="summary">{plugin.summary}</p>
      <dl class="metrics">
        {plugin.metrics.map((metric) => (
          <div key={metric.label}>
            <dt>{metric.label}</dt>
            <dd>
              {metric.value}
              <small>{metric.unit}</small>
            </dd>
          </div>
        ))}
      </dl>
      <footer>
        <span>检查于 {time(plugin.checked_at)}</span>
        {plugin.url && (
          <a href={plugin.url} target="_blank" rel="noopener noreferrer">
            打开服务 <span aria-hidden="true">↗</span>
          </a>
        )}
      </footer>
    </article>
  );
}

function App() {
  const [data, setData] = useState<Status | null>(null);
  const [page, setPage] = useState<"overview" | "services" | "about">(
    "overview",
  );
  const [error, setError] = useState("");
  const [needsAuth, setNeedsAuth] = useState(false);
  const [token, setToken] = useState("");
  const [draft, setDraft] = useState("");
  const [busy, setBusy] = useState(false);
  const [tick, setTick] = useState(0);
  const [clock, setClock] = useState(Date.now());
  const sequence = useRef(0);
  useEffect(() => {
    let active = true;
    let timer: ReturnType<typeof setTimeout>;
    let controller: AbortController | null = null;
    const fetchStatus = async () => {
      if (document.hidden) return;
      const current = ++sequence.current;
      controller?.abort();
      controller = new AbortController();
      const requestController = controller;
      const deadline = setTimeout(() => requestController.abort(), 8000);
      setBusy(true);
      try {
        const response = await fetch("/api/status", {
          headers: token ? { Authorization: `Bearer ${token}` } : {},
          signal: controller.signal,
          cache: "no-store",
        });
        if (!active || current !== sequence.current) return;
        if (response.status === 401) {
          setNeedsAuth(true);
          setData(null);
          setError(token ? "访问令牌不正确，请重新输入。" : "");
          return;
        }
        if (!response.ok)
          throw new Error(`状态接口返回 HTTP ${response.status}`);
        const next = (await response.json()) as Status;
        if (!active || current !== sequence.current) return;
        setData(next);
        setError("");
        setNeedsAuth(false);
        document.title = next.title;
      } catch (e) {
        if (active && current === sequence.current)
          setError(
            e instanceof Error && e.name !== "AbortError"
              ? e.message
              : "状态请求超时或连接中断",
          );
      } finally {
        clearTimeout(deadline);
        if (active && current === sequence.current) {
          setBusy(false);
          timer = setTimeout(fetchStatus, 5000);
        }
      }
    };
    const visibility = () => {
      if (!document.hidden) {
        clearTimeout(timer);
        void fetchStatus();
      }
    };
    document.addEventListener("visibilitychange", visibility);
    void fetchStatus();
    return () => {
      active = false;
      controller?.abort();
      clearTimeout(timer);
      document.removeEventListener("visibilitychange", visibility);
    };
  }, [token, tick]);
  useEffect(() => {
    const timer = setInterval(() => setClock(Date.now()), 1000);
    return () => clearInterval(timer);
  }, []);

  const now = Math.floor(clock / 1000);
  const plugins = (data?.plugins ?? []).map((plugin) =>
    plugin.checked_at && now - plugin.checked_at > plugin.stale_after_secs
      ? {
          ...plugin,
          health: "unknown" as Health,
          summary: "采集数据已过期，等待下一次检查",
        }
      : plugin,
  );
  const checks = plugins.filter((plugin) => plugin.kind !== "link");
  const healthy = checks.filter((plugin) => plugin.health === "healthy").length;
  const failed = checks.filter(
    (plugin) => plugin.health === "failed" || plugin.health === "degraded",
  ).length;
  const stale = !!data && now - data.generated_at > 15;
  const displayed =
    page === "overview" ? plugins.filter((plugin) => plugin.visible) : plugins;

  return (
    <div class="layout">
      <aside class="sidebar">
        <a
          class="brand"
          href="#"
          onClick={(e) => {
            e.preventDefault();
            setPage("overview");
          }}
        >
          <span class="brand-mark">≋</span>
          <span>
            router<span class="brand-light">console</span>
            <small>YOUR NETWORK, AT A GLANCE</small>
          </span>
        </a>
        <div class="nav-label">工作空间</div>
        <nav aria-label="主导航">
          <button
            class={page === "overview" ? "selected" : ""}
            onClick={() => setPage("overview")}
          >
            <Icon>▦</Icon>运行概览<span>01</span>
          </button>
          <button
            class={page === "services" ? "selected" : ""}
            onClick={() => setPage("services")}
          >
            <Icon>◫</Icon>全部服务
            <span>{plugins.length.toString().padStart(2, "0")}</span>
          </button>
          <button
            class={page === "about" ? "selected" : ""}
            onClick={() => setPage("about")}
          >
            <Icon>ⓘ</Icon>关于控制台
          </button>
        </nav>
        <div class="sidebar-note">
          <span class="tiny-dot" />
          独立运行 · 只读模式<p>控制台停止不会停止你的网络服务。</p>
        </div>
        <div class="sidebar-bottom">
          <span>ROUTER CONSOLE</span>
          <span>v{data?.version ?? "0.1.0"}</span>
        </div>
      </aside>
      <main>
        <header class="topbar">
          <span>
            工作空间 <b>/</b>{" "}
            {page === "overview"
              ? "运行概览"
              : page === "services"
                ? "全部服务"
                : "关于控制台"}
          </span>
          <span class="topbar-right">
            <span class={`tiny-dot ${error || stale ? "warning-dot" : ""}`} />
            {needsAuth
              ? "等待认证"
              : data
                ? error || stale
                  ? "连接异常"
                  : "已连接"
                : "连接中"}
            <time>
              {new Date(clock).toLocaleTimeString("zh-CN", { hour12: false })}
            </time>
          </span>
        </header>
        <div class="content">
          <div class="page-heading">
            <div>
              <div class="eyebrow">NETWORK WORKSPACE</div>
              <h1>
                {page === "overview"
                  ? "网络运行概览"
                  : page === "services"
                    ? "你的服务"
                    : "轻量、独立、可组合。"}
              </h1>
              <p>
                {page === "overview"
                  ? "把基础网络与代理分开看，让每一项状态都有依据。"
                  : page === "services"
                    ? "查看所有已启用插件，包括未在首页展示的服务。"
                    : "为低资源 Linux 路由器构建的模块化控制台。"}
              </p>
            </div>
            <button
              class="refresh"
              disabled={busy}
              onClick={() => setTick((value) => value + 1)}
            >
              <span aria-hidden="true">↻</span>
              {busy ? "刷新中" : "刷新状态"}
            </button>
          </div>
          {error && (
            <div class="alert" role="alert">
              {error} {data && "以下为上次采集数据，请留意检查时间。"}
            </div>
          )}
          {stale && (
            <div class="alert" role="status">
              控制台连接已过期，当前数据可能不再反映服务状态。
            </div>
          )}
          {needsAuth ? (
            <section class="auth-panel">
              <div class="eyebrow">PROTECTED WORKSPACE</div>
              <h2>连接你的控制台</h2>
              <p>输入服务端配置的访问令牌。令牌仅保存在当前页面内存中。</p>
              <form
                onSubmit={(e) => {
                  e.preventDefault();
                  if (draft.trim()) {
                    setToken(draft.trim());
                    setDraft("");
                    setTick((value) => value + 1);
                  }
                }}
              >
                <label for="token">访问令牌</label>
                <input
                  id="token"
                  type="password"
                  value={draft}
                  onInput={(e) => setDraft(e.currentTarget.value)}
                  autoComplete="off"
                  required
                />
                <button class="primary" type="submit">
                  连接控制台 →
                </button>
              </form>
            </section>
          ) : page === "about" ? (
            <section class="about card">
              <h2>Router Console</h2>
              <p>
                Rust / Axum 后端，Preact
                静态前端。采集任务共享缓存，不随浏览器数量重复运行。
              </p>
              <h3>插件与展示</h3>
              <p>
                插件可分别启用或隐藏。HTTP 探测只证明接口可达；DNS
                探测直接查询指定服务器的 A 记录；入口插件不声明服务健康。
              </p>
              <h3>第一版边界</h3>
              <p>
                只读状态与服务入口。没有配置编辑、服务重启或订阅更新操作。网络、DNS
                和订阅任务由部署侧独立管理。系统指标需要 Linux。
              </p>
              <a
                href="https://github.com/hizolaw/router-console"
                target="_blank"
                rel="noopener noreferrer"
              >
                查看项目源码 ↗
              </a>
            </section>
          ) : (
            <>
              <section class="stats" aria-label="状态摘要">
                <div>
                  <span>已启用插件</span>
                  <strong>
                    {data ? plugins.length.toString().padStart(2, "0") : "—"}
                  </strong>
                  <small>按配置独立装配</small>
                </div>
                <div>
                  <span>探测正常</span>
                  <strong class="mint">
                    {data ? healthy.toString().padStart(2, "0") : "—"}
                    <em>/ {checks.length}</em>
                  </strong>
                  <small>入口插件不计入</small>
                </div>
                <div>
                  <span>需要关注</span>
                  <strong class={failed ? "amber" : ""}>
                    {data ? failed.toString().padStart(2, "0") : "—"}
                  </strong>
                  <small>异常或降级的探测</small>
                </div>
                <div>
                  <span>最近同步</span>
                  <strong class="sync-time">
                    {time(data?.generated_at ?? null)}
                  </strong>
                  <small>页面可见时每 5 秒刷新</small>
                </div>
              </section>
              <div class="section-heading">
                <h2>
                  {page === "overview" ? "服务与资源" : "全部插件"}
                  <span>{displayed.length}</span>
                </h2>
                <span>状态来自独立采集任务</span>
              </div>
              {!data ? (
                <div class="empty" role="status">
                  正在连接控制台，读取服务状态…
                </div>
              ) : displayed.length ? (
                <section class="cards">
                  {displayed.map((plugin) => (
                    <Card key={plugin.id} plugin={plugin} />
                  ))}
                </section>
              ) : (
                <div class="empty">
                  暂无{page === "overview" ? "可展示的" : "已启用的"}
                  插件。请在服务端配置中启用插件。
                </div>
              )}
              <div class="footnote">
                <span>◎</span>
                <p>
                  基础网络应独立于代理运行。控制台呈现探测结果，不接管网络故障恢复。
                </p>
              </div>
            </>
          )}
          <footer class="page-footer">
            <span>轻量连接，清晰可见。</span>
            <span>RUST + PREACT · v{data?.version ?? "0.1.0"}</span>
            {token && (
              <button
                onClick={() => {
                  setToken("");
                  setData(null);
                }}
              >
                退出
              </button>
            )}
          </footer>
        </div>
      </main>
    </div>
  );
}
render(<App />, document.getElementById("app")!);
