# Headers every haryx.wecoap.com response carries. nginx drops server-level add_header in any
# location that sets its own, so each location that adds headers includes this instead.
# Named .inc so aaPanel's *.conf include does not pull it into `http {}`.
add_header Strict-Transport-Security "max-age=31536000" always;
add_header X-Content-Type-Options "nosniff" always;
add_header Referrer-Policy "strict-origin-when-cross-origin" always;
