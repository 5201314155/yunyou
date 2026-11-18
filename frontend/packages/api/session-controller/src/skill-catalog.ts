/** Session-addressed, cold-readable skill catalog Remote. */

import type { Context } from '@deepseek-ai/cordis'
import type {} from '@deepseek-ai/dsh-agent-preset-registry/types'
import { SessionQueryError } from '@deepseek-ai/dsh-session-query'
import type { SessionId } from '@deepseek-ai/dsh-session'
import type { SkillRegistry, SkillSummary } from '@deepseek-ai/dsh-skill'
import { isUserInvocable } from '@deepseek-ai/dsh-skill'
import type {} from '@deepseek-ai/dsh-working-directory'
import type { ScopeKey } from '@deepseek-ai/dsh-scope'
import { Remote, RemoteError, TypertRemoteService } from '@deepseek-ai/dsh-typert-protocol'
import type {
  SessionSkillGetRequest, SessionSkillGetValue, SkillListRequest, SkillListValue,
} from './types.ts'

declare module '@deepseek-ai/cordis' {
  interface Context {
    /** Host owner of the Session-addressed `skills` Remote namespace. */
    sessionSkillCatalog: SessionSkillCatalog
  }
}

/** One Session composition's resolved skill-reading view. */
interface SkillCompositionView {
  readonly cwd: string
  readonly scope: ScopeKey | undefined
  readonly registry: SkillRegistry
  /** Disposer for the standing preset scope; undefined for a live agent. */
  readonly lease?: { readonly key: ScopeKey } & AsyncDisposable
}

/** Host service backing `ctx.remote.skills` without activating a cold Agent. */
export class SessionSkillCatalog extends TypertRemoteService {
  static inject = ['agents', 'sessionQuery', 'typert', 'workingDirectory']

  /** @param ctx - Host context carrying Session reads and optional skill/preset services. */
  constructor(ctx: Context) {
    super(ctx, 'sessionSkillCatalog', { namespace: 'skills' })
  }

  /**
   * List the user-invocable skills visible to one Session composition.
   * @param request - Session identity whose cwd and preset select the catalog view.
   * @param signal - caller lifetime carried by the Remote transport; admitted catalog reads retain their existing completion semantics.
   * @returns user-invocable skill metadata without loading skill bodies.
   * @throws RemoteError when the Session cannot be inspected or no registry can serve it.
   */
  @Remote
  async list(request: SkillListRequest, signal: AbortSignal): Promise<SkillListValue> {
    const { sessionId } = request
    await using view = await this.compositionView(sessionId, signal)
    try {
      const skills = (await view.registry.list({ cwd: view.cwd, scope: view.scope })).filter(isUserInvocable)
      return {
        skills: skills.map(skill => ({
          name: skill.name,
          ...skill.path === undefined ? {} : { path: skill.path },
          description: skill.description,
          ...skill.whenToUse === undefined ? {} : { whenToUse: skill.whenToUse },
          modelInvocable: skill.invocation.modelInvocable,
        })),
      }
    } catch (error: unknown) {
      throw new RemoteError('gateway/internal', `skill listing failed: ${String(error)}`, {})
    }
  }

  /**
   * Read one user-invocable skill's full body for human viewing and explicit
   * fill-back (web.v1 §9.2: a skill is a read-only catalog entry, never an
   * executable RPC).
   * @param request - Session identity selecting the composition and the skill id.
   * @returns the skill metadata and its raw body text.
   * @throws RemoteError when the skill is absent from the Session's composition.
   */
  @Remote
  async get(request: SessionSkillGetRequest): Promise<SessionSkillGetValue> {
    const { sessionId, id } = request
    await using view = await this.compositionView(sessionId, undefined)
    const matches = await this.visibleSkills(view)
    const match = matches.find(skill => skill.name === id)
      ?? matches.find(skill => skill.path !== undefined && skill.path.endsWith(`/${id}`))
    if (match === undefined) {
      throw new RemoteError('skills/unknown-id', `skill "${id}" is not visible in session "${sessionId}"`, { id })
    }
    const definition = await view.registry.get(match.name, { cwd: view.cwd, scope: view.scope })
    if (definition === undefined) {
      throw new RemoteError('skills/unknown-id', `skill "${id}" could not be loaded`, { id })
    }
    return {
      name: definition.name,
      description: definition.description,
      body: definition.content,
    }
  }

  /** Resolve a live or standing preset scope without creating an Agent. */
  private async scopeFor(
    agentPreset: string | undefined,
  ): Promise<({ key: ScopeKey } & AsyncDisposable) | undefined> {
    const presets = this.ctx.get('agentPresets')
    if (presets === undefined) return undefined
    try {
      return await presets.acquireScope(agentPreset)
    } catch {
      // An unknown or unusable recorded preset falls back to the global registry.
      return undefined
    }
  }

  /** Resolve one Session's cwd, viewing scope, and skill registry. */
  private async compositionView(sessionId: SessionId, signal: AbortSignal | undefined): Promise<SkillCompositionView & AsyncDisposable> {
    let cwd: string
    let agentPreset: string | undefined
    try {
      using observation = await this.ctx.sessionQuery.observeSession(sessionId)
      if (observation.projections === undefined) {
        throw new Error('skill catalog requires a projected Session observation')
      }
      cwd = observation.projections.values.workingDirectory
        ?? observation.header.cwd
        ?? this.ctx.workingDirectory.defaultDirectory
      agentPreset = observation.projections.values.agentPreset ?? undefined
      const live = this.ctx.agents.get(sessionId)
      if (live !== undefined && signal !== undefined) cwd = await this.ctx.workingDirectory.ensure(live, signal)
    } catch (error: unknown) {
      if (error instanceof SessionQueryError
        && error.code === 'SESSION_QUERY_SESSION_NOT_FOUND') {
        throw new RemoteError('session/not-found', `session "${sessionId}" not found`, { sessionId })
      }
      throw new RemoteError(
        'gateway/internal',
        `session "${sessionId}" could not be inspected: ${String(error)}`,
        {},
      )
    }

    const live = this.ctx.agents.get(sessionId)
    const presets = this.ctx.get('agentPresets')
    const scoped = live === undefined ? undefined : presets?.serviceFor(live, 'skills')
    const registry = scoped ?? this.ctx.get('skills')
    if (registry === undefined) {
      throw new RemoteError(
        'gateway/internal',
        'skill registry is absent: neither this session\'s agent preset nor the host composition mounts @deepseek-ai/dsh-skill',
        {},
      )
    }
    const lease = live === undefined ? await this.scopeFor(agentPreset) : undefined
    const view: SkillCompositionView = {
      cwd,
      scope: live ?? lease?.key,
      registry,
      ...(lease === undefined ? {} : { lease }),
    }
    // `await using` releases the standing preset scope when the read settles.
    return Object.assign(view, {
      [Symbol.asyncDispose]: () => lease?.[Symbol.asyncDispose]() ?? Promise.resolve(),
    })
  }

  /** The user-invocable summaries of one composition. */
  private async visibleSkills(view: SkillCompositionView): Promise<readonly SkillSummary[]> {
    return (await view.registry.list({ cwd: view.cwd, scope: view.scope })).filter(isUserInvocable)
  }
}

export default SessionSkillCatalog
