<?php
// Synthetic service fixture: no client-project source or configuration.
namespace App\Service;
use Psr\Cache\CacheItemPoolInterface;
use Symfony\Component\HttpFoundation\RequestStack;
class EntryFetchThrottle
{
    private const MAX_ATTEMPTS = 60;
    private const MAX_GLOBAL_ATTEMPTS = 6000;
    private const WINDOW_SECONDS = 60;
    public function __construct(private CacheItemPoolInterface $cache, private RequestStack $requestStack) {}
    public function allow(): bool
    {
        return $this->consume('car_financing_entry_fetch_' . sha1($this->callerId()), self::MAX_ATTEMPTS)
            && $this->consume('car_financing_entry_fetch_global', self::MAX_GLOBAL_ATTEMPTS);
    }
    private function callerId(): string
    {
        return $this->requestStack->getCurrentRequest()?->getClientIp() ?? 'unknown-caller';
    }
    private function consume(string $key, int $maxAttempts): bool
    {
        $item = $this->cache->getItem($key);
        [$attempts, $windowStartedAt] = $item->isHit() && is_array($item->get())
            ? $item->get()
            : [0, time()];
        $remaining = self::WINDOW_SECONDS - (time() - $windowStartedAt);
        if ($remaining <= 0) {
            $attempts = 0;
            $windowStartedAt = time();
            $remaining = self::WINDOW_SECONDS;
        }
        if ($attempts >= $maxAttempts) {
            return false;
        }
        $item->set([$attempts + 1, $windowStartedAt]);
        $item->expiresAfter($remaining);
        $this->cache->save($item);
        return true;
    }
}
