import z from '@deepseek-ai/schemastery';
import { credentialRef } from '@deepseek-ai/dsh-credentials';
import { launchEnvironmentOf } from '@deepseek-ai/dsh-launch-environment';
import { deepSeekApiKeyConfigured, rejectDeepSeekAccountToken, resolveDeepSeekAccountToken, resolveDeepSeekApiKey, } from '@deepseek-ai/dsh-llm-deepseek';
import { plainAccountOptions, resolveAccountOptions, } from '@deepseek-ai/dsh-llm-deepseek-account';
import { plainOptions as plainOfficialOptions, resolveAdapterOptions as resolveOfficialOptions, } from '@deepseek-ai/dsh-llm-deepseek-api-key';
import { DeepSeekFetchApiProvider } from "./fetch.js";
import { DEEPSEEK_DEFAULT_COUNT, DEEPSEEK_SEARCH_REQUESTS_PER_SECOND, DEEPSEEK_WEB_API_BASE_URL, DeepSeekSearchApiProvider, } from "./search.js";
import { isDeepSeekWebAuthReady, } from "./transport.js";
export { DEEPSEEK_FETCH_API_PROVIDER_ID, DeepSeekFetchApiProvider } from "./fetch.js";
export { DEEPSEEK_DEFAULT_COUNT, DEEPSEEK_SEARCH_API_PROVIDER_ID, DEEPSEEK_SEARCH_REQUESTS_PER_SECOND, DEEPSEEK_WEB_API_BASE_URL, DeepSeekSearchApiProvider, mapSearchResponse, } from "./search.js";
/** Cordis plugin name used by loader diagnostics. */
export const name = 'web-deepseek-api';
/** The web seam both providers register into. */
export const inject = ['web'];
export const Config = z.object({
    count: z.number().step(1).min(1).max(50).default(DEEPSEEK_DEFAULT_COUNT).volatile(),
    searchRequestsPerSecond: z.number().step(1).min(1).default(DEEPSEEK_SEARCH_REQUESTS_PER_SECOND).volatile(),
    sendOriginalQuestion: z.boolean().default(true).volatile(),
    searchBilling: z.union(['auto', 'account', 'api-key']).default('auto').volatile(),
    fetchBilling: z.union(['auto', 'account', 'api-key']).default('auto').volatile(),
    searchBaseURL: z.string().volatile(),
    searchApiKeyEnv: z.string().role('credential-ref').volatile(),
    fetchBaseURL: z.string().volatile(),
    fetchApiKeyEnv: z.string().role('credential-ref').volatile(),
});
/** Path prefix of the Search and Fetch APIs under a DeepSeek origin. */
const TOOLS_PATH = '/v1/tools';
/** Credential reference of the official DeepSeek API key. */
const DEEPSEEK_API_KEY = credentialRef('DEEPSEEK_API_KEY');
/** Provider id of the DeepSeek account route. */
const ACCOUNT_PROVIDER = 'deepseek-account';
/** Provider id of the DeepSeek API-key route. */
const OFFICIAL_PROVIDER = 'deepseek-official';
/** Loader package that registers each model route. */
const ROUTE_PACKAGES = {
    [OFFICIAL_PROVIDER]: '@deepseek-ai/dsh-llm-deepseek-api-key',
    [ACCOUNT_PROVIDER]: '@deepseek-ai/dsh-llm-deepseek-account',
};
/** Display names, configuration prefix, and operation path of each capability. */
const CAPABILITY = {
    search: { subject: 'Search', prefix: 'search', path: '/search' },
    fetch: { subject: 'Fetch', prefix: 'fetch', path: '/fetch' },
};
/**
 * Read the text of the latest human prompt, the question that triggered the
 * current turn. Injected user-role context carries another source kind.
 * @param session - the initiating Session, when the search runs inside one.
 * @returns the prompt's joined text blocks, or undefined without a prompt.
 */
function latestUserQuestion(session) {
    const prompt = session?.deriveMessages().findLast(message => message.role === 'user' && message.source.kind === 'user');
    if (prompt === undefined)
        return undefined;
    return prompt.content.flatMap(block => block.type === 'text' ? [block.text] : []).join('\n');
}
/**
 * Register the Search API and Fetch API providers over one live settings section. Each operation reads one settings
 * snapshot and the route of the initiating Session's latest model request; each `ready()` call reads one settings
 * snapshot and the route of the model request being prepared. Both resolve the same ordered credentials.
 * @param ctx - plugin context owning both registrations.
 * @param config - live settings, read once per operation.
 */
export function apply(ctx, config) {
    const own = (capability) => capability === 'search'
        ? { baseURL: config.searchBaseURL.get(), ref: config.searchApiKeyEnv.get(), billing: config.searchBilling.get() }
        : { baseURL: config.fetchBaseURL.get(), ref: config.fetchApiKeyEnv.get(), billing: config.fetchBilling.get() };
    const target = (capability, llmProvider) => resolveTarget(ctx, capability, own(capability), llmProvider);
    const current = (capability) => target(capability, ctx.get('agents')?.currentInitiator()?.session.requestContext()?.provider);
    const ready = (capability) => (llmProvider) => isDeepSeekWebAuthReady(target(capability, llmProvider).auth, CAPABILITY[capability].path);
    const searchApi = new DeepSeekSearchApiProvider(() => ({
        ...current('search'),
        count: config.count.get(),
        requestsPerSecond: config.searchRequestsPerSecond.get(),
        ...config.sendOriginalQuestion.get() ? {
            originalQuestion: () => latestUserQuestion(ctx.get('agents')?.currentInitiator()?.session),
        } : {},
    }));
    const fetchApi = new DeepSeekFetchApiProvider(() => current('fetch'));
    ctx.web.registerSearchProvider({
        id: searchApi.id,
        description: searchApi.description,
        available: () => searchApi.available(),
        ready: ready('search'),
        search: (request, signal) => searchApi.search(request, signal),
    });
    ctx.web.registerFetchProvider({
        id: fetchApi.id,
        description: fetchApi.description,
        available: () => fetchApi.available(),
        ready: ready('fetch'),
        fetch: (request, signal) => fetchApi.fetch(request, signal),
    });
}
/**
 * Resolve the ordered credentials and endpoint guidance of one capability without reading credential values. Route
 * endpoints use the origin of the route's resolved Messages base URL with `/v1/tools`.
 * @param ctx - context with the model, loader, credential, and account services.
 * @param capability - the capability being resolved.
 * @param own - the capability's own base URL and credential reference, where empty values count as unset, and its
 *   billing.
 * @param llmProvider - provider route of the model request the operation serves.
 * @returns the authentication sources and the settings endpoint failures name.
 */
function resolveTarget(ctx, capability, own, llmProvider) {
    const { subject, prefix } = CAPABILITY[capability];
    const guidance = (custom) => ({ subject, plugin: name, custom, baseURLField: `${prefix}BaseURL`, apiKeyEnvField: `${prefix}ApiKeyEnv` });
    const ownBaseURL = nonEmpty(own.baseURL), ownRef = nonEmpty(own.ref);
    if (ownBaseURL !== undefined || ownRef !== undefined) {
        const ref = ownRef === undefined ? DEEPSEEK_API_KEY : credentialRef(ownRef);
        const json = JSON.stringify(ref);
        return { guidance: guidance(true), auth: {
                sources: [apiKeySource(ctx, ownBaseURL ?? DEEPSEEK_WEB_API_BASE_URL, ref)],
                missingCredentialMessage: `DeepSeek ${subject} API sends the API key under ${json} to its own endpoint, and no key is`
                    + ` configured for ${json}. Guide the user to export ${json} in the launching environment, or to correct`
                    + ` ${name}.${prefix}ApiKeyEnv.`,
            } };
    }
    const rule = own.billing !== 'auto' ? own.billing
        : llmProvider === ACCOUNT_PROVIDER ? 'account-conversation'
            : llmProvider === OFFICIAL_PROVIDER ? 'official-conversation' : 'other-conversation';
    const sources = ruleSources(ctx, capability, rule);
    const [ref = DEEPSEEK_API_KEY] = sources.flatMap(row => row.kind === 'api-key' && typeof row.key !== 'string' ? [row.key.ref] : []);
    return { guidance: guidance(false), auth: { sources, missingCredentialMessage: missingMessage(capability, rule, ref) } };
}
/**
 * The credentials one rule tries, in order.
 * @param ctx - context with the model, loader, credential, and account services.
 * @param capability - the capability whose diagnostics misconfigured sources name.
 * @param rule - the rule.
 * @returns the sources; the account source is absent in an account conversation without a mounted account route.
 */
function ruleSources(ctx, capability, rule) {
    const accountConfig = routeConfig(ctx, ACCOUNT_PROVIDER);
    switch (rule) {
        case 'account-conversation': return accountConfig === undefined
            ? [officialSource(ctx, capability)] : [accountSource(ctx, capability, accountConfig), officialSource(ctx, capability)];
        case 'official-conversation':
        case 'api-key': return [officialSource(ctx, capability)];
        case 'other-conversation': return [officialSource(ctx, capability), accountSource(ctx, capability, accountConfig)];
        case 'account': return [accountSource(ctx, capability, accountConfig)];
        /* v8 ignore next -- the switch is exhaustive. */
        default: return assertNever(rule);
    }
}
/**
 * The API key of the active DeepSeek API-key route, sent to its origin, or `DEEPSEEK_API_KEY` sent to the official base
 * without that route.
 * @returns the source, or a misconfigured source when the route's resolver rejects its settings.
 */
function officialSource(ctx, capability) {
    const config = routeConfig(ctx, OFFICIAL_PROVIDER);
    if (config === undefined)
        return apiKeySource(ctx, DEEPSEEK_WEB_API_BASE_URL, DEEPSEEK_API_KEY);
    let resolved;
    try {
        resolved = resolveOfficialOptions(plainOfficialOptions(config), launchEnvironmentOf(ctx));
    }
    catch (error) {
        return misconfigured(capability, OFFICIAL_PROVIDER, error);
    }
    return apiKeySource(ctx, toolsBase(resolved.baseURL), resolved.apiKeyEnv);
}
/**
 * The account token sent to the origin of the account route, or without that route to the origin of its default base
 * URL, including `DEEPSEEK_BASE_URL`. The account service releases the token only for destinations it allows.
 * @param config - the active account route's settings, or undefined for the defaults.
 * @returns the source, or a misconfigured source when the route's resolver rejects its settings.
 */
function accountSource(ctx, capability, config) {
    let baseURL;
    try {
        baseURL = resolveAccountOptions(config === undefined ? {} : plainAccountOptions(config), launchEnvironmentOf(ctx)).protocol.baseURL;
    }
    catch (error) {
        return misconfigured(capability, ACCOUNT_PROVIDER, error);
    }
    return {
        kind: 'account', baseURL: toolsBase(baseURL),
        resolveToken: endpoint => resolveDeepSeekAccountToken(ctx, endpoint),
        rejectToken: token => rejectDeepSeekAccountToken(ctx, token),
    };
}
/** The key under `ref`, read through the shared DeepSeek credential helpers, sent to `baseURL`. */
function apiKeySource(ctx, baseURL, ref) {
    return { kind: 'api-key', baseURL, key: {
            ref, configured: () => deepSeekApiKeyConfigured(ctx, ref), resolve: () => resolveDeepSeekApiKey(ctx, ref),
        } };
}
/**
 * The source of a credential whose model route settings do not resolve.
 * @param capability - the capability that stops.
 * @param route - the route whose settings failed.
 * @param error - the route resolver's failure.
 * @returns the source whose diagnostic names the route and both recoveries.
 */
function misconfigured(capability, route, error) {
    const { subject, prefix } = CAPABILITY[capability];
    const provider = route === ACCOUNT_PROVIDER ? 'DeepSeek account model provider' : 'DeepSeek API-key model provider';
    const credential = route === ACCOUNT_PROVIDER ? 'the DeepSeek account sign-in' : 'that provider\'s API key';
    return { kind: 'misconfigured', message: `DeepSeek ${subject} API sends ${credential} to the origin of the ${provider}, and`
            + ` that provider's settings do not resolve: ${errorMessage(error)}. ${subject} did not send the request. Guide the user`
            + ' to correct that provider\'s settings on the Models page or in the deployment configuration, or to set'
            + ` ${name}.${prefix}BaseURL and ${name}.${prefix}ApiKeyEnv so ${subject} uses its own endpoint.` };
}
/**
 * Model-facing diagnostic for an operation billed by `rule` that has no credential to send.
 * @param capability - the capability that stops.
 * @param rule - the rule that ordered its credentials.
 * @param keyRef - the reference of the API key the rule tries, or `DEEPSEEK_API_KEY` when it tries none.
 * @returns the diagnostic naming the missing credentials and their recoveries.
 */
function missingMessage(capability, rule, keyRef) {
    const { subject, prefix } = CAPABILITY[capability];
    const ref = JSON.stringify(keyRef);
    const addKey = `add the DeepSeek API key on the Models page or export ${ref} in the launching environment`;
    const billing = `${name}.${prefix}Billing`;
    const settings = 'on the Web search and fetch settings page';
    switch (rule) {
        case 'account-conversation':
            return `DeepSeek ${subject} API uses the DeepSeek account sign-in of this conversation, then the DeepSeek API key, and`
                + ` neither is available: no signed-in DeepSeek account is available, and no key is configured for ${ref}. Guide the`
                + ` user to sign in to DeepSeek, or to ${addKey}.`;
        case 'official-conversation':
            return `DeepSeek ${subject} API uses the DeepSeek API key, as this conversation's model provider does, and no key is`
                + ` configured for ${ref}. Guide the user to ${addKey}, or to sign in to DeepSeek and set ${billing} to "account"`
                + ` ${settings}.`;
        case 'other-conversation':
            return `DeepSeek ${subject} API uses the DeepSeek API key, then the DeepSeek account sign-in, and neither is available:`
                + ` no key is configured for ${ref}, and no signed-in DeepSeek account is available. Guide the user to ${addKey}, or`
                + ' to sign in to DeepSeek.';
        case 'account':
            return `DeepSeek ${subject} API bills only the signed-in DeepSeek account, and no signed-in DeepSeek account is`
                + ` available. ${subject} did not send the request. Guide the user to sign in to DeepSeek, or to set ${billing} to`
                + ` "api-key" or "auto" ${settings}.`;
        case 'api-key':
            return `DeepSeek ${subject} API bills only the DeepSeek API key, and no key is configured for ${ref}. Guide the user to`
                + ` ${addKey}, or to set ${billing} to "account" or "auto" ${settings}.`;
        /* v8 ignore next -- the switch is exhaustive. */
        default: return assertNever(rule);
    }
}
/** The message of a thrown value. */
function errorMessage(error) {
    return error instanceof Error ? error.message : String(error);
}
/**
 * Find the effective config of the active loader entry that registers `provider`, as the model provider directory
 * names it. A route registered by another package, nested settings, or an inactive entry counts as absent.
 */
function routeConfig(ctx, provider) {
    const llm = ctx.get('llm'), loader = ctx.get('loader');
    if (llm === undefined || loader === undefined || !llm.listProviders().some(row => row.id === provider))
        return undefined;
    const directory = llm.listConfigurableProviders().find(row => row.provider === provider);
    if (directory === undefined || directory.settingsPath.length !== 0)
        return undefined;
    const entry = [...loader.entries()].find(row => row.options.id === directory.settingsNs && row.options.name === ROUTE_PACKAGES[provider]);
    if (entry?.fiber?.state !== 2 /* FiberState.ACTIVE */)
        return undefined;
    return entry.fiber.effectiveConfig;
}
/** The Search and Fetch API base under the origin of a Messages base URL. */
function toolsBase(messagesBaseURL) {
    return `${new URL(messagesBaseURL).origin}${TOOLS_PATH}`;
}
/** A string with content, or undefined for an absent or empty value. */
function nonEmpty(value) {
    return value !== undefined && value.length > 0 ? value : undefined;
}
/* v8 ignore start -- reached only by a value outside a closed union, which the compiler rejects. */
/** Reject an unhandled member of a closed union. */
function assertNever(value) {
    throw new Error(`web-deepseek-api: unexpected value ${String(value)}`);
}
/* v8 ignore stop */
//# sourceMappingURL=index.js.map