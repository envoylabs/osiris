(ns shop.store-test
  (:require [clojure.test :refer [deftest is]]
            [shop.store :as store]))

(deftest loads (is (store/load)))
