/** 团队中心文案（web.v1 team/tasks 域产品面；8 席同键 satisfies 锁齐性）。 */

/** 字典命名空间。 */
export const NS = 'teamCenter'

/** 简体中文（键集真源）。 */
export const zh = {
  'panel.label': '团队',
  'page.title': '团队中心',
  'page.refresh': '刷新',
  'page.unavailable': '团队接口未连接。此部署尚未开通 team/tasks Web 通道，开通后此页显示真实名册与任务板。',
  'roster.title': '名册',
  'roster.empty': '暂无成员。用「拉人」组建你的团队。',
  'roster.spawn': '拉人',
  'roster.online': '在线',
  'roster.offline': '离线',
  'roster.busy': '忙碌',
  'roster.idle': '空闲',
  'roster.kick': '踢出',
  'roster.message': '发消息',
  'tasks.title': '共享任务',
  'tasks.empty': '暂无任务。创建一条任务并指派给成员。',
  'tasks.blockedBy': '被 {count} 条前置任务阻塞',
  'task.claim': '认领',
  'task.submit': '提交',
  'task.review.approve': '通过',
  'task.review.reject': '驳回',
  'task.reassign': '改派',
  'state.pending': '待认领',
  'state.claimed': '已认领',
  'state.submitted': '待审查',
  'state.reviewing': '审查中',
  'state.approved': '已通过',
  'state.rejected': '已驳回',
  'spawn.title': '拉起成员',
  'spawn.name': '成员名',
  'spawn.description': '职责描述',
  'spawn.prompt': '初始任务',
  'spawn.context': '上下文',
  'spawn.context.fresh': '空白（全新上下文）',
  'spawn.context.fork': '继承（lead 对话摘要注入）',
  'spawn.forkNote': 'fork 为摘要降级形态，不是完整会话复制。',
  'spawn.submit': '拉起',
  'spawn.cancel': '取消',
  'message.title': '发消息给 {name}',
  'message.body': '消息内容',
  'message.send': '发送',
  'message.cancel': '取消',
  'confirm.kick.title': '踢出成员 {name}？',
  'confirm.kick.body': '该成员的运行会被停止，信箱保留；名册保留其元数据。',
  'confirm.kick.acknowledge': '我确认要踢出该成员',
  'confirm.review.title': '审查任务 {title}？',
  'confirm.review.body': '审查裁决不可由同一人对自己提交的任务做出（服务端强制）；通过后任务进入终态。',
  'confirm.review.acknowledge': '我确认本次审查裁决',
  'confirm.submit': '确认',
  'confirm.cancel': '取消',
  'close': '关闭',
  'opFailed': '操作失败：{message}',
  'reassign.title': '改派任务 {title}',
  'reassign.owner': '新负责人（须在册）',
  'reassign.confirm': '改派',
} satisfies Record<string, string>

/** 团队中心键联合。 */
export type TeamCenterKey = keyof typeof zh


export { en } from './locales/en.ts'
export { ja } from './locales/ja.ts'
export { es } from './locales/es.ts'
export { fr } from './locales/fr.ts'
export { de } from './locales/de.ts'
export { ru } from './locales/ru.ts'
export { pt } from './locales/pt.ts'
