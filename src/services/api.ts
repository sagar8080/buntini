import { command } from '../bridge';
import type { ServiceId, ServicesOp, ServiceStatus } from './model';

export const callServices = <T>(op: ServicesOp) => command<T>('services_call', { op });

/** Every service's state. Asking counts as watching, so the services are polled while Settings
    shows them. */
export const serviceStatuses = () => callServices<ServiceStatus[]>({ op: 'status' });

/** Saves a key in the Keychain, or removes it when empty. */
export const saveServiceKey = (service: ServiceId, key: string) => callServices<ServiceStatus[]>({ op: 'saveKey', service, key });

/** Shows a service on the island again. */
export const showService = (service: ServiceId) => callServices<ServiceStatus[]>({ op: 'show', service });
