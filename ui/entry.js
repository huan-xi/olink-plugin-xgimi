// 与宿主共用 @olink-home/ui（同追觅页），经 /plugin-runtime facade 加载。
import { createElement, useCallback, useEffect, useRef, useState } from '/plugin-runtime/react.js'
import { usePluginContext } from '/plugin-runtime/plugin-sdk.js'
import {
  Button,
  Dialog,
  DialogContent,
  DialogFooterBar,
  DialogHeaderBar,
  Input,
  Label,
  PageHeader,
} from '/plugin-runtime/olink-ui.js'

function unwrapBody(body) {
  return body?.data ?? body
}

function channelLabel(channel) {
  return channel === 'lan' ? '局域网' : '云端'
}

function deviceSubtitle(d) {
  const ch = channelLabel(d.channel)
  if (d.channel === 'lan' && d.host) return `${ch} · ${d.host}`
  if (d.host) return `${ch} · ${d.host}`
  return ch
}

function HomePage() {
  const { domain, version, pluginFetch, toast } = usePluginContext()
  const [devices, setDevices] = useState([])
  const [account, setAccount] = useState({ logged_in: false })
  const [cloudDevices, setCloudDevices] = useState([])
  const [selected, setSelected] = useState({})
  const [lanOpen, setLanOpen] = useState(false)
  const [lanTaskId, setLanTaskId] = useState(null)
  const [lanPhase, setLanPhase] = useState('idle')
  const [lanMessage, setLanMessage] = useState('')
  const [lanDevices, setLanDevices] = useState([])
  const [lanSelected, setLanSelected] = useState({})
  const lanPollRef = useRef(null)
  const [busy, setBusy] = useState(false)
  const [loginOpen, setLoginOpen] = useState(false)
  const [loginForm, setLoginForm] = useState({ mobile: '', code: '' })
  const [detailOpen, setDetailOpen] = useState(false)
  const [detailLoading, setDetailLoading] = useState(false)
  const [detail, setDetail] = useState(null)
  const [detailFallback, setDetailFallback] = useState('')
  const [error, setError] = useState(null)

  const loadDevices = useCallback(async () => {
    const body = await pluginFetch('/devices')
    const data = unwrapBody(body)
    setDevices(Array.isArray(data?.devices) ? data.devices : [])
  }, [pluginFetch])

  const loadAccount = useCallback(async () => {
    const body = await pluginFetch('/account/status')
    const data = unwrapBody(body)
    setAccount(data || { logged_in: false })
  }, [pluginFetch])

  const refresh = useCallback(async () => {
    setError(null)
    await Promise.all([loadDevices(), loadAccount()])
  }, [loadDevices, loadAccount])

  useEffect(() => {
    refresh().catch((e) => toast(String(e?.message ?? e), { type: 'error' }))
  }, [refresh, toast])

  const openLogin = () => {
    setLoginForm({ mobile: '', code: '' })
    setError(null)
    setLoginOpen(true)
  }

  const sendSms = async () => {
    setBusy(true)
    setError(null)
    try {
      const body = await pluginFetch('/account/sms', {
        method: 'POST',
        headers: { 'content-type': 'application/json' },
        body: JSON.stringify({ mobile: loginForm.mobile.trim() }),
      })
      const data = unwrapBody(body)
      if (data?.ok === false) throw new Error(data.message || '发码失败')
      toast('验证码已发送', { type: 'success' })
    } catch (e) {
      setError(String(e?.message ?? e))
    } finally {
      setBusy(false)
    }
  }

  const doLogin = async () => {
    setBusy(true)
    setError(null)
    try {
      const body = await pluginFetch('/account/login', {
        method: 'POST',
        headers: { 'content-type': 'application/json' },
        body: JSON.stringify({
          mobile: loginForm.mobile.trim(),
          code: loginForm.code.trim(),
        }),
      })
      const data = unwrapBody(body)
      if (data?.ok === false) throw new Error(data.message || '登录失败')
      setLoginOpen(false)
      await refresh()
      toast('登录成功', { type: 'success' })
    } catch (e) {
      setError(String(e?.message ?? e))
    } finally {
      setBusy(false)
    }
  }

  const doLogout = async () => {
    if (!confirm('退出极米账号？已接入设备不会自动删除。')) return
    setBusy(true)
    try {
      await pluginFetch('/account/logout', { method: 'POST' })
      setCloudDevices([])
      setSelected({})
      await refresh()
      toast('已退出登录', { type: 'success' })
    } catch (e) {
      toast(String(e?.message ?? e), { type: 'error' })
    } finally {
      setBusy(false)
    }
  }

  const discover = async () => {
    setBusy(true)
    setError(null)
    try {
      const body = await pluginFetch('/devices/discover')
      const data = unwrapBody(body)
      if (data?.ok === false) throw new Error(data.message || '发现失败')
      const list = Array.isArray(data?.devices) ? data.devices : []
      setCloudDevices(list)
      const next = {}
      for (const d of list) {
        if (!d.imported) next[d.sn] = true
      }
      setSelected(next)
      toast(`发现 ${list.length} 台云设备`, { type: 'success' })
    } catch (e) {
      setError(String(e?.message ?? e))
    } finally {
      setBusy(false)
    }
  }

  const doImport = async () => {
    const items = cloudDevices
      .filter((d) => selected[d.sn])
      .map((d) => ({
        sn: d.sn,
        name: d.name,
        item_code: d.item_code || undefined,
      }))
    if (items.length === 0) {
      toast('请先勾选设备', { type: 'warning' })
      return
    }
    setBusy(true)
    try {
      const body = await pluginFetch('/devices/import', {
        method: 'POST',
        headers: { 'content-type': 'application/json' },
        body: JSON.stringify({ items }),
      })
      const data = unwrapBody(body)
      if (data?.ok === false && !(data?.imported || []).length) {
        throw new Error(data.message || '导入失败')
      }
      await loadDevices()
      await discover()
      toast(`已导入 ${(data?.imported || []).length} 台`, { type: 'success' })
    } catch (e) {
      toast(String(e?.message ?? e), { type: 'error' })
    } finally {
      setBusy(false)
    }
  }

  const stopLanPoll = useCallback(() => {
    if (lanPollRef.current) {
      clearInterval(lanPollRef.current)
      lanPollRef.current = null
    }
  }, [])

  const cancelLanSearch = useCallback(
    async (taskId) => {
      const id = taskId || lanTaskId
      if (!id) return
      try {
        await pluginFetch(`/devices/lan/search/${encodeURIComponent(id)}/cancel`, {
          method: 'POST',
        })
      } catch (_) {
        /* ignore */
      }
    },
    [lanTaskId, pluginFetch],
  )

  const pollLanOnce = useCallback(
    async (taskId) => {
      const body = await pluginFetch(`/devices/lan/search/${encodeURIComponent(taskId)}`)
      const data = unwrapBody(body)
      if (data?.ok === false) throw new Error(data.message || '查询搜索状态失败')
      const list = Array.isArray(data?.devices) ? data.devices : []
      setLanDevices(list)
      setLanPhase(data?.phase || 'running')
      setLanMessage(data?.message || '')
      setLanSelected((prev) => {
        const next = { ...prev }
        for (const d of list) {
          if (next[d.pid] === undefined && !d.imported && d.support?.light !== false) {
            next[d.pid] = true
          }
        }
        return next
      })
      if (data?.phase === 'completed' || data?.phase === 'failed' || data?.phase === 'cancelled') {
        stopLanPoll()
      }
      return data
    },
    [pluginFetch, stopLanPoll],
  )

  const startLanSearch = useCallback(async () => {
    stopLanPoll()
    setLanDevices([])
    setLanSelected({})
    setLanPhase('pending')
    setLanMessage('准备搜索…')
    setBusy(true)
    try {
      const body = await pluginFetch('/devices/lan/search', { method: 'POST' })
      const data = unwrapBody(body)
      if (data?.ok === false) throw new Error(data.message || '启动局域网搜索失败')
      const taskId = data?.task_id
      if (!taskId) throw new Error('未返回 task_id')
      setLanTaskId(taskId)
      await pollLanOnce(taskId)
      lanPollRef.current = setInterval(() => {
        pollLanOnce(taskId).catch((e) => {
          stopLanPoll()
          setLanPhase('failed')
          setLanMessage(String(e?.message ?? e))
        })
      }, 1000)
    } catch (e) {
      setLanPhase('failed')
      setLanMessage(String(e?.message ?? e))
      toast(String(e?.message ?? e), { type: 'error' })
    } finally {
      setBusy(false)
    }
  }, [pluginFetch, pollLanOnce, stopLanPoll, toast])

  const openLanDialog = () => {
    setLanOpen(true)
    startLanSearch()
  }

  const closeLanDialog = async () => {
    stopLanPoll()
    const id = lanTaskId
    setLanOpen(false)
    if (id && (lanPhase === 'pending' || lanPhase === 'running')) {
      await cancelLanSearch(id)
    }
    setLanTaskId(null)
    setLanPhase('idle')
    setLanMessage('')
  }

  const restartLanSearch = async () => {
    if (lanTaskId) await cancelLanSearch(lanTaskId)
    await startLanSearch()
  }

  const doLanImport = async () => {
    const items = lanDevices
      .filter((d) => lanSelected[d.pid] && d.support?.light !== false && !d.imported)
      .map((d) => ({
        pid: d.pid,
        name: d.name,
        host: d.host,
        product_name: d.product_name || undefined,
      }))
    if (items.length === 0) {
      toast('请先勾选可接入的局域网设备', { type: 'warning' })
      return
    }
    setBusy(true)
    try {
      const body = await pluginFetch('/devices/lan/import', {
        method: 'POST',
        headers: { 'content-type': 'application/json' },
        body: JSON.stringify({ items }),
      })
      const data = unwrapBody(body)
      if (data?.ok === false && !(data?.imported || []).length) {
        throw new Error(data.message || '局域网导入失败')
      }
      await loadDevices()
      toast(`已导入 ${(data?.imported || []).length} 台局域网设备`, { type: 'success' })
      await closeLanDialog()
    } catch (e) {
      toast(String(e?.message ?? e), { type: 'error' })
    } finally {
      setBusy(false)
    }
  }

  useEffect(() => () => stopLanPoll(), [stopLanPoll])

  const remove = async (id) => {
    if (!confirm(`删除设备 ${id}？`)) return
    try {
      await pluginFetch(`/devices/${encodeURIComponent(id)}`, { method: 'DELETE' })
      await loadDevices()
    } catch (e) {
      toast(String(e?.message ?? e), { type: 'error' })
    }
  }

  const openDetail = async (device) => {
    setDetailFallback(device.name || device.id)
    setDetail(null)
    setDetailOpen(true)
    setDetailLoading(true)
    try {
      const body = await pluginFetch(`/devices/${encodeURIComponent(device.id)}/detail`)
      const data = unwrapBody(body)
      if (data?.ok === false) throw new Error(data.message || '获取详情失败')
      setDetail(data)
    } catch (e) {
      toast(String(e?.message ?? e), { type: 'error' })
      setDetailOpen(false)
    } finally {
      setDetailLoading(false)
    }
  }

  const toggle = async (device) => {
    try {
      await pluginFetch('/olink/v1/write', {
        method: 'POST',
        headers: { 'content-type': 'application/json' },
        body: JSON.stringify({
          device_key: device.id,
          attr_id: 'power-state',
          value: !device.on,
        }),
      })
      await loadDevices()
    } catch (e) {
      toast(String(e?.message ?? e), { type: 'error' })
    }
  }

  const loggedIn = !!account.logged_in

  return createElement(
    PageHeader,
    {
      title: '极米',
      action: createElement(
        'div',
        { className: 'plg-xgimi-actions' },
        createElement(
          Button,
          {
            variant: 'outline',
            onClick: openLanDialog,
            disabled: busy || lanOpen,
            className: 'gap-2',
          },
          '局域网发现',
        ),
        createElement(
          Button,
          { onClick: openLogin, disabled: busy, className: 'gap-2' },
          '+ 新增账号',
        ),
      ),
    },
    createElement(
      'p',
      { className: 'plg-xgimi-sub' },
      `短信登录 · 局域网/云设备勾选导入 · 神灯 / 投影仪 · ${domain} @ ${version}`,
    ),

    createElement(
      'div',
      { className: 'plg-xgimi-tip' },
      '登录极米账号后可拉取云端设备并勾选接入；局域网发现无需账号，接入走本地协议，与账号设备相互独立。控制能力按机型逐步补齐，当前以神灯为主。',
    ),

    error && !loginOpen
      ? createElement('div', { className: 'plg-xgimi-error' }, error)
      : null,

    createElement(
      'div',
      { className: 'plg-xgimi-card' },
      createElement(
        'h3',
        { className: 'plg-xgimi-card-title' },
        loggedIn ? '已登录账号（1）' : '已登录账号',
      ),
      !loggedIn
        ? createElement(
            'p',
            { className: 'plg-xgimi-muted' },
            '暂无账号。可先点右上角「局域网发现」弹窗搜索接入神灯；或「新增账号」用极米 App 手机号登录后发现云设备。',
          )
        : createElement(
            'div',
            { className: 'plg-xgimi-account-row' },
            createElement(
              'div',
              null,
              createElement('p', { className: 'plg-xgimi-name' }, account.mobile || '已登录'),
              createElement(
                'p',
                { className: 'plg-xgimi-muted' },
                `uid ${account.uid || '—'} · openId ${account.open_id ? `${String(account.open_id).slice(0, 8)}…` : '—'}`,
              ),
            ),
            createElement(
              'div',
              { className: 'plg-xgimi-actions' },
              createElement(
                Button,
                {
                  variant: 'outline',
                  size: 'sm',
                  disabled: busy,
                  onClick: discover,
                },
                '发现云设备',
              ),
              createElement(
                Button,
                {
                  variant: 'outline',
                  size: 'sm',
                  disabled: busy,
                  onClick: doLogout,
                  className: 'plg-xgimi-danger',
                },
                '退出',
              ),
            ),
          ),
    ),

    loggedIn && cloudDevices.length > 0
      ? createElement(
          'div',
          { className: 'plg-xgimi-card' },
          createElement(
            'div',
            { className: 'plg-xgimi-card-head' },
            createElement('h3', { className: 'plg-xgimi-card-title' }, '云设备'),
            createElement(
              Button,
              { size: 'sm', disabled: busy, onClick: doImport },
              '导入勾选',
            ),
          ),
          createElement(
            'ul',
            { className: 'plg-xgimi-list' },
            ...cloudDevices.map((d) =>
              createElement(
                'li',
                { key: d.sn, className: 'plg-xgimi-row' },
                createElement(
                  'label',
                  { className: 'plg-xgimi-check' },
                  createElement('input', {
                    type: 'checkbox',
                    checked: !!selected[d.sn],
                    disabled: !!d.imported || busy,
                    onChange: () =>
                      setSelected((prev) => ({ ...prev, [d.sn]: !prev[d.sn] })),
                  }),
                  createElement(
                    'div',
                    null,
                    createElement(
                      'div',
                      { className: 'plg-xgimi-name' },
                      `${d.name} (${d.sn})`,
                    ),
                    createElement(
                      'div',
                      { className: 'plg-xgimi-muted' },
                      d.imported ? '已接入' : d.item_code || '—',
                    ),
                  ),
                ),
              ),
            ),
          ),
        )
      : null,

    createElement(
      'div',
      { className: 'plg-xgimi-card' },
      createElement(
        'div',
        { className: 'plg-xgimi-card-head' },
        createElement(
          'div',
          null,
          createElement('h3', { className: 'plg-xgimi-card-title' }, '已接入设备'),
          createElement(
            'p',
            { className: 'plg-xgimi-muted' },
            `共 ${devices.length} 台`,
          ),
        ),
      ),
      devices.length === 0
        ? createElement(
            'p',
            { className: 'plg-xgimi-muted' },
            '暂无设备。可点「局域网发现」弹窗勾选导入，或登录后点「发现云设备」导入。',
          )
        : createElement(
            'ul',
            { className: 'plg-xgimi-list' },
            ...devices.map((d) =>
              createElement(
                'li',
                { key: d.id, className: 'plg-xgimi-row' },
                createElement(
                  'div',
                  { className: 'plg-xgimi-row-meta' },
                  createElement('div', { className: 'plg-xgimi-name' }, d.name || d.id),
                  createElement(
                    'div',
                    { className: 'plg-xgimi-muted' },
                    `${d.id} · ${deviceSubtitle(d)}`,
                  ),
                ),
                createElement(
                  'div',
                  { className: 'plg-xgimi-actions' },
                  createElement(
                    Button,
                    {
                      size: 'sm',
                      variant: 'outline',
                      onClick: () => openDetail(d),
                    },
                    '详情',
                  ),
                  createElement(
                    Button,
                    {
                      size: 'sm',
                      variant: d.on ? 'default' : 'outline',
                      onClick: () => toggle(d),
                    },
                    d.on ? '关闭' : '打开',
                  ),
                  createElement(
                    Button,
                    {
                      size: 'sm',
                      variant: 'outline',
                      className: 'plg-xgimi-danger',
                      onClick: () => remove(d.id),
                    },
                    '删除',
                  ),
                ),
              ),
            ),
          ),
    ),

    createElement(
      Dialog,
      {
        open: lanOpen,
        onOpenChange: (open) => {
          if (!open) closeLanDialog()
          else setLanOpen(true)
        },
      },
      createElement(
        DialogContent,
        {
          className:
            'flex max-h-[90vh] w-[90vw] max-w-[480px] flex-col gap-0 overflow-hidden rounded-xl border border-slate-200/60 p-0 shadow-[0_4px_24px_rgba(15,23,42,0.12)]',
        },
        createElement(DialogHeaderBar, {
          title: '搜索局域网设备',
          subtitle: '设备保存在局域网，无需账号',
        }),
        createElement(
          'div',
          { className: 'plg-xgimi-dialog-body' },
          createElement(
            'div',
            { className: 'plg-xgimi-muted', style: { marginBottom: 12 } },
            lanPhase === 'running' || lanPhase === 'pending'
              ? lanMessage || '搜索中…'
              : lanPhase === 'completed'
                ? lanMessage || '搜索结束'
                : lanMessage || '',
          ),
          lanDevices.length === 0
            ? createElement(
                'div',
                { className: 'plg-xgimi-muted' },
                lanPhase === 'running' || lanPhase === 'pending'
                  ? '正在搜索附近设备…'
                  : '未发现设备',
              )
            : createElement(
                'ul',
                { className: 'plg-xgimi-list' },
                ...lanDevices.map((d) => {
                  const lightOk = d.support?.light !== false
                  const disabled = !!d.imported || !lightOk || busy
                  let sub = d.host || '—'
                  if (d.imported) sub = `已接入 · ${d.host || d.id || ''}`
                  else if (!lightOk) sub = '暂不支持接入'
                  else if (d.product_name) sub = `${d.product_name} · ${d.host || '—'}`
                  return createElement(
                    'li',
                    { key: d.pid, className: 'plg-xgimi-row' },
                    createElement(
                      'label',
                      { className: 'plg-xgimi-check' },
                      createElement('input', {
                        type: 'checkbox',
                        checked: !!lanSelected[d.pid],
                        disabled,
                        onChange: () =>
                          setLanSelected((prev) => ({
                            ...prev,
                            [d.pid]: !prev[d.pid],
                          })),
                      }),
                      createElement(
                        'div',
                        null,
                        createElement(
                          'div',
                          { className: 'plg-xgimi-name' },
                          `${d.name || d.pid} (${d.pid})`,
                        ),
                        createElement('div', { className: 'plg-xgimi-muted' }, sub),
                      ),
                    ),
                  )
                }),
              ),
        ),
        createElement(
          DialogFooterBar,
          null,
          createElement(
            Button,
            { variant: 'outline', disabled: busy, onClick: closeLanDialog },
            '取消',
          ),
          lanPhase === 'completed' || lanPhase === 'failed' || lanPhase === 'cancelled'
            ? createElement(
                Button,
                {
                  variant: 'outline',
                  disabled: busy,
                  onClick: restartLanSearch,
                },
                '重新搜索',
              )
            : null,
          createElement(
            Button,
            {
              disabled:
                busy ||
                !lanDevices.some(
                  (d) =>
                    lanSelected[d.pid] && d.support?.light !== false && !d.imported,
                ),
              onClick: doLanImport,
              className: 'bg-blue-600 text-white hover:bg-blue-700',
            },
            `添加所选 (${
              lanDevices.filter(
                (d) =>
                  lanSelected[d.pid] && d.support?.light !== false && !d.imported,
              ).length
            })`,
          ),
        ),
      ),
    ),

    createElement(
      Dialog,
      {
        open: loginOpen,
        onOpenChange: (open) => {
          setLoginOpen(open)
          if (!open) {
            setLoginForm({ mobile: '', code: '' })
            setError(null)
          }
        },
      },
      createElement(
        DialogContent,
        {
          className:
            'flex max-h-[90vh] w-[90vw] max-w-[440px] flex-col gap-0 overflow-hidden rounded-xl border border-slate-200/60 p-0 shadow-[0_4px_24px_rgba(15,23,42,0.12)]',
        },
        // DialogHeaderBar：title/subtitle 是 props；children 是标题右侧槽，勿把标题塞进去
        createElement(DialogHeaderBar, {
          title: '新增极米账号',
          subtitle: '使用极米 App 手机号验证码登录',
        }),
        createElement(
          'div',
          { className: 'plg-xgimi-dialog-body' },
          error && loginOpen
            ? createElement('div', { className: 'plg-xgimi-error' }, error)
            : null,
          createElement(
            'div',
            { className: 'plg-xgimi-field' },
            createElement(Label, { htmlFor: 'xg-mobile' }, '手机号'),
            createElement(Input, {
              id: 'xg-mobile',
              type: 'tel',
              value: loginForm.mobile,
              placeholder: '1xxxxxxxxxx',
              disabled: busy,
              onChange: (e) =>
                setLoginForm((prev) => ({ ...prev, mobile: e.target.value })),
            }),
          ),
          createElement(
            'div',
            { className: 'plg-xgimi-field' },
            createElement(Label, { htmlFor: 'xg-code' }, '验证码'),
            createElement(
              'div',
              { className: 'plg-xgimi-code-row' },
              createElement(Input, {
                id: 'xg-code',
                value: loginForm.code,
                placeholder: '短信验证码',
                disabled: busy,
                onChange: (e) =>
                  setLoginForm((prev) => ({ ...prev, code: e.target.value })),
              }),
              createElement(
                Button,
                {
                  type: 'button',
                  variant: 'outline',
                  disabled: busy || !loginForm.mobile.trim(),
                  onClick: sendSms,
                },
                '发送验证码',
              ),
            ),
          ),
        ),
        createElement(
          DialogFooterBar,
          null,
          createElement(
            Button,
            { variant: 'outline', disabled: busy, onClick: () => setLoginOpen(false) },
            '取消',
          ),
          createElement(
            Button,
            {
              disabled: busy || !loginForm.mobile.trim() || !loginForm.code.trim(),
              onClick: doLogin,
              className: 'bg-blue-600 text-white hover:bg-blue-700',
            },
            busy ? '登录中…' : '登录',
          ),
        ),
      ),
    ),

    createElement(
      Dialog,
      {
        open: detailOpen,
        onOpenChange: (open) => {
          setDetailOpen(open)
          if (!open) {
            setDetail(null)
            setDetailFallback('')
          }
        },
      },
      createElement(
        DialogContent,
        {
          className:
            'flex max-h-[90vh] w-[90vw] max-w-[640px] flex-col gap-0 overflow-hidden rounded-xl border border-slate-200/60 p-0 shadow-[0_4px_24px_rgba(15,23,42,0.12)]',
        },
        createElement(DialogHeaderBar, {
          title: detailLoading
            ? '加载中…'
            : detail?.name || detailFallback || '设备详情',
          subtitle: detail?.did || (detail?.id ? `xgimi.${detail.id}` : undefined),
        }),
        createElement(
          'div',
          { className: 'plg-xgimi-dialog-body plg-xgimi-detail-body' },
          detailLoading
            ? createElement('p', { className: 'plg-xgimi-muted' }, '正在查询设备详情…')
            : !detail
              ? createElement('p', { className: 'plg-xgimi-muted' }, '暂无数据')
              : createElement(
                  'div',
                  { className: 'plg-xgimi-detail' },
                  createElement(
                    'div',
                    { className: 'plg-xgimi-detail-badges' },
                    createElement(
                      'span',
                      { className: 'plg-xgimi-badge plg-xgimi-badge-schema' },
                      detail.schema_id || 'light',
                    ),
                    createElement(
                      'span',
                      {
                        className: detail.host_registered
                          ? 'plg-xgimi-badge plg-xgimi-badge-ok'
                          : 'plg-xgimi-badge plg-xgimi-badge-err',
                      },
                      detail.host_registered ? '已注册 Host' : '未注册 Host',
                    ),
                    detail.channel
                      ? createElement(
                          'span',
                          { className: 'plg-xgimi-badge' },
                          channelLabel(detail.channel),
                        )
                      : null,
                  ),
                  !detail.host_registered && detail.register_error
                    ? createElement(
                        'div',
                        { className: 'plg-xgimi-error' },
                        String(detail.register_error),
                      )
                    : null,
                  createElement(
                    'section',
                    { className: 'plg-xgimi-detail-section' },
                    createElement('h3', { className: 'plg-xgimi-detail-h' }, '设备信息'),
                    createElement(
                      'dl',
                      { className: 'plg-xgimi-detail-grid' },
                      ...[
                        ['制造商', detail.manufacturer],
                        ['型号', detail.model],
                        ['序列号', detail.id],
                        [
                          'Host',
                          detail.host?.trim()
                            ? detail.host
                            : channelLabel(detail.channel),
                        ],
                        ['通道', channelLabel(detail.channel)],
                        ['DID', detail.did],
                      ].map(([k, v]) =>
                        createElement(
                          'div',
                          { key: k, className: 'plg-xgimi-detail-cell' },
                          createElement('dt', null, k),
                          createElement('dd', null, String(v ?? '—')),
                        ),
                      ),
                    ),
                  ),
                  createElement(
                    'section',
                    { className: 'plg-xgimi-detail-section' },
                    createElement('h3', { className: 'plg-xgimi-detail-h' }, '服务与属性'),
                    ...(Array.isArray(detail.services) ? detail.services : []).map((svc) =>
                      createElement(
                        'div',
                        {
                          key: svc.service_type,
                          className: 'plg-xgimi-detail-svc',
                        },
                        createElement(
                          'p',
                          { className: 'plg-xgimi-name' },
                          svc.service_type || 'service',
                        ),
                        createElement(
                          'ul',
                          { className: 'plg-xgimi-detail-attrs' },
                          ...(Array.isArray(svc.attributes) ? svc.attributes : []).map(
                            (attr) =>
                              createElement(
                                'li',
                                { key: attr.attr_type },
                                createElement('span', null, attr.attr_type),
                                createElement(
                                  'span',
                                  { className: 'plg-xgimi-muted' },
                                  attr.access || '',
                                ),
                              ),
                          ),
                        ),
                      ),
                    ),
                  ),
                ),
        ),
        createElement(
          DialogFooterBar,
          null,
          createElement(
            Button,
            {
              variant: 'outline',
              onClick: () => setDetailOpen(false),
            },
            '关闭',
          ),
        ),
      ),
    ),
  )
}

export const routes = [{ index: true, element: createElement(HomePage) }]
