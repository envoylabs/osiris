(ns shop.core
  (:require [shop.store :as store]
            [shop.wire]))

(defn run [] (store/load))
